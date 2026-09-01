use async_trait::async_trait;
use serde_json::json;

use crate::agent::tool::{OfficeTool, ToolArtifact, ToolContext, ToolResult};

/// 飞书访问身份：用户 token / 应用 token
enum FeishuIdentity {
    User(String), // user_access_token
    App(String),  // tenant_access_token
}

/// 飞书 API 权限类错误码（触发交互式授权流程：对话中引导用户授权）
fn is_feishu_permission_code(code: i64) -> bool {
    matches!(code,
        99991003 | // 应用未开通该权限
        99991400 | 99991419 | // 无权限访问
        99991661 | 99991662 | 99991663 | // scope 未授权系列
        99991668 | 99991669 | 99991670 | 99991671 | 99991672) // 无权限/角色限制系列
}

/// API 错误 → ToolResult：权限类错误转交互式授权（needs_auth → 前端弹授权 + agent 引导用户），
/// 其余错误保持原样，避免权限不足时 agent 沉默无回复。
fn feishu_err_to_result(err: anyhow::Error, scope: &str) -> ToolResult {
    let s = format!("{err:#}");
    let permission = (|| {
        if let Some(pos) = s.find("code=") {
            let rest = &s[pos + 5..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(code) = digits.parse::<i64>() {
                return is_feishu_permission_code(code);
            }
        }
        s.contains("权限") || s.contains("无权限") || s.contains("Forbidden")
            || s.contains("Access denied") || s.contains(" 403") || s.contains("(403")
    })();
    if permission {
        ToolResult::err_needs_auth(scope)
    } else {
        ToolResult::err(s)
    }
}

/// 简化 JSON 数组 → Markdown 表格（产物供用户在右边栏/我的文件直接阅读，不再是一坨 JSON）
fn json_array_to_markdown(rows: &[serde_json::Value], title: &str) -> String {
    let mut md = format!("# {title}\n\n");
    if rows.is_empty() {
        md.push_str("（无内容）\n");
        return md;
    }
    let mut cols: Vec<String> = Vec::new();
    if let Some(first) = rows.first().and_then(|r| r.as_object()) {
        cols = first.keys().take(6).cloned().collect();
    }
    if cols.is_empty() {
        md.push_str("```json\n");
        md.push_str(&serde_json::to_string_pretty(rows).unwrap_or_default());
        md.push_str("\n```\n");
        return md;
    }
    md.push_str(&format!("| {} |\n", cols.join(" | ")));
    md.push_str(&format!("| {} |\n", cols.iter().map(|_| "---").collect::<Vec<_>>().join(" | ")));
    for row in rows {
        let obj = match row.as_object() {
            Some(o) => o,
            None => {
                md.push_str(&format!("| {} |\n", row.to_string().replace('|', "\\|")));
                continue;
            }
        };
        let cells: Vec<String> = cols.iter().map(|k| {
            obj.get(k).map(|v| match v {
                serde_json::Value::String(x) => x.clone(),
                serde_json::Value::Null => String::new(),
                other => other.to_string(),
            }).unwrap_or_default()
            .replace('|', "\\|")
            .replace('\n', " ")
        }).collect();
        md.push_str(&format!("| {} |\n", cells.join(" | ")));
    }
    md
}

/// 需要授权的结果：缺少的 scope
#[derive(Debug, Clone)]
pub struct NeedsAuth {
    pub scope: String,
}

/// 获取 tenant_access_token（应用身份，用于飞书文档/表格/日历 API）
async fn get_tenant_access_token() -> anyhow::Result<String> {
    let cfg = crate::config::config();
    if cfg.feishu_app_id.is_empty() || cfg.feishu_app_secret.is_empty() {
        return Err(anyhow::anyhow!("未配置飞书应用（FEISHU_APP_ID/FEISHU_APP_SECRET）"));
    }
    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .post("https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal")
        .json(&json!({
            "app_id": cfg.feishu_app_id,
            "app_secret": cfg.feishu_app_secret,
        }))
        .send().await?.error_for_status()?.json().await?;
    let code = resp.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code != 0 {
        return Err(anyhow::anyhow!(
            "获取飞书 token 失败(code={}): {}",
            code,
            resp.get("msg").and_then(|v| v.as_str()).unwrap_or("unknown")
        ));
    }
    resp.get("tenant_access_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("tenant_access_token 缺失"))
}

/// 刷新 user_access_token（用 refresh_token 换新 token）
async fn refresh_user_token(refresh_token: &str) -> anyhow::Result<serde_json::Value> {
    let cfg = crate::config::config();
    let client = reqwest::Client::new();
    let resp = client
        .post("https://open.feishu.cn/open-apis/authen/v1/oidc/refresh_access_token")
        .json(&json!({
            "grant_type": "refresh_token",
            "client_id": cfg.feishu_app_id,
            "client_secret": cfg.feishu_app_secret,
            "refresh_token": refresh_token,
        }))
        .send().await?;
    let json: serde_json::Value = resp.json().await?;
    let code = json.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code != 0 {
        return Err(anyhow::anyhow!(
            "刷新飞书 token 失败: {}",
            json.get("msg").and_then(|v| v.as_str()).unwrap_or("unknown")
        ));
    }
    Ok(json)
}

/// 统一飞书访问解析：
/// 1. 用户有 user token 且未过期 → 用用户身份
/// 2. user token 过期但有 refresh_token → 先刷新再返回
/// 3. 无 user token → 用应用身份（tenant token）兜底
/// 4. 需要指定 scope 时，若用户未授权该 scope → 返回 NeedsAuth
///
/// 说明：应用身份（tenant）能访问「应用被加为协作者」或「组织内公开」的资源；
/// 用户身份（user）能访问用户个人可访问的资源。按需授权时，若资源需要用户 scope
/// 而用户未授权，则返回 NeedsAuth 引导前端弹授权。
async fn resolve_feishu_access(
    ctx: &ToolContext,
    required_scope: Option<&str>,
) -> Result<FeishuIdentity, NeedsAuth> {
    // 读用户的飞书 token
    let pool = crate::state::db_pool();
    if let Ok(Some(settings)) = crate::db::settings_repo::find_by_user(&pool, &ctx.user_id).await {
        let ft = settings.feishu_token.clone();
        let now = chrono::Utc::now().timestamp();

        // 有 user token
        if !ft.user_access_token.is_empty() {
            // 检查是否过期
            if ft.expires_at > now {
                // 检查 scope 是否满足
                if let Some(scope) = required_scope {
                    let has = ft.scopes.split_whitespace().any(|s| s == scope);
                    if !has {
                        return Err(NeedsAuth { scope: scope.to_string() });
                    }
                }
                return Ok(FeishuIdentity::User(ft.user_access_token));
            }
            // 过期 → 尝试用 refresh_token 刷新
            if !ft.refresh_token.is_empty() && ft.refresh_expires_at > now {
                if let Ok(new_json) = refresh_user_token(&ft.refresh_token).await {
                    let new_access = new_json.get("access_token").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let new_refresh = new_json.get("refresh_token").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let expires_in = new_json.get("expires_in").and_then(|v| v.as_i64()).unwrap_or(7200);
                    if !new_access.is_empty() {
                        // 回写新的 token
                        let mut s = settings.clone();
                        s.feishu_token.user_access_token = new_access.clone();
                        if !new_refresh.is_empty() {
                            s.feishu_token.refresh_token = new_refresh;
                        }
                        s.feishu_token.expires_at = now + expires_in;
                        let _ = crate::db::settings_repo::save_for_user(&pool, &ctx.user_id, &s).await;
                        return Ok(FeishuIdentity::User(new_access));
                    }
                }
            }
        }
    }

    // 兜底：应用身份（tenant token）
    if let Ok(app_token) = get_tenant_access_token().await {
        return Ok(FeishuIdentity::App(app_token));
    }

    Err(NeedsAuth { scope: required_scope.unwrap_or("").to_string() })
}

// ============ 飞书文档读取工具 ============

pub struct FeishuDocReadTool;

#[async_trait]
impl OfficeTool for FeishuDocReadTool {
    fn name(&self) -> &str {
        "feishu_doc_read"
    }

    fn description(&self) -> &str {
        "读取飞书云文档（docx）的纯文本内容。输入文档的 document_id（从文档 URL 中 /docx/ 后的一段）。"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "document_id": { "type": "string", "description": "飞书文档 ID，形如 MDE0xxx（从 https://xxx.feishu.cn/docx/XXX 中提取 XXX）" }
            },
            "required": ["document_id"]
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn produces_artifact(&self) -> bool {
        true
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let doc_id = input.get("document_id").and_then(|v| v.as_str()).unwrap_or("").trim();
        if doc_id.is_empty() {
            return ToolResult::err("document_id 不能为空");
        }
        // 统一解析飞书访问身份（用户 token 优先 → 应用兜底 → 需授权信号）
        let identity = match resolve_feishu_access(ctx, Some("docx:document:readonly")).await {
            Ok(id) => id,
            Err(needs) => return ToolResult::err_needs_auth(&needs.scope),
        };
        let token = match &identity {
            FeishuIdentity::User(t) | FeishuIdentity::App(t) => t.clone(),
        };
        match do_read_doc(&token, doc_id).await {
            Ok(text) => {
                let artifact = ToolArtifact {
                    kind: "markdown".into(),
                    title: format!("飞书文档 · {doc_id}"),
                    content: json!({ "type": "markdown", "markdown": text, "source": "feishu_doc" }),
                };
                ToolResult {
                    success: true,
                    data: Some(json!({ "document_id": doc_id, "content": text })),
                    error: None,
                    artifacts: Some(vec![artifact]),
                    observation: format!("飞书文档内容如下：\n{text}"),
                    needs_auth: None,
                    continue_loop: None,
                }
            }
            Err(e) => feishu_err_to_result(e, "docx:document:readonly"),
        }
    }
}

async fn do_read_doc(token: &str, document_id: &str) -> anyhow::Result<String> {
    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .get(format!(
            "https://open.feishu.cn/open-apis/docx/v1/documents/{document_id}/raw_content"
        ))
        .header("Authorization", format!("Bearer {token}"))
        .send().await?.error_for_status()?.json().await?;
    let code = resp.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code != 0 {
        return Err(anyhow::anyhow!(
            "飞书文档读取失败(code={}): {}",
            code,
            resp.get("msg").and_then(|v| v.as_str()).unwrap_or("unknown")
        ));
    }
    resp.get("data")
        .and_then(|d| d.get("content"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("文档内容为空"))
}

// ============ 飞书多维表格查询工具 ============

pub struct FeishuBitableQueryTool;

#[async_trait]
impl OfficeTool for FeishuBitableQueryTool {
    fn name(&self) -> &str {
        "feishu_bitable_query"
    }

    fn description(&self) -> &str {
        "查询飞书多维表格（Bitable）的记录。输入 app_token 和 table_id（从多维表格 URL 中提取：/base/APP_TOKEN?table=TABLE_ID）。"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "app_token": { "type": "string", "description": "多维表格 app_token（URL /base/ 后的一段）" },
                "table_id": { "type": "string", "description": "数据表 table_id（URL ?table= 后的值）" },
                "page_size": { "type": "integer", "description": "返回记录数，默认 20", "minimum": 1, "maximum": 100 }
            },
            "required": ["app_token", "table_id"]
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn produces_artifact(&self) -> bool {
        true
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let app_token = input.get("app_token").and_then(|v| v.as_str()).unwrap_or("").trim();
        let table_id = input.get("table_id").and_then(|v| v.as_str()).unwrap_or("").trim();
        let page_size = input.get("page_size").and_then(|v| v.as_u64()).unwrap_or(20).clamp(1, 100) as usize;
        if app_token.is_empty() || table_id.is_empty() {
            return ToolResult::err("app_token 和 table_id 不能为空");
        }
        let identity = match resolve_feishu_access(ctx, Some("bitable:app:readonly")).await {
            Ok(id) => id,
            Err(needs) => return ToolResult::err_needs_auth(&needs.scope),
        };
        let token = match &identity {
            FeishuIdentity::User(t) | FeishuIdentity::App(t) => t.clone(),
        };
        match do_query_bitable(&token, app_token, table_id, page_size).await {
            Ok(records) => {
                let md_str = json_array_to_markdown(&records, "飞书多维表格记录");
                let artifact = ToolArtifact {
                    kind: "markdown".into(),
                    title: format!("飞书表格记录 · {table_id}"),
                    content: json!({ "type": "markdown", "markdown": md_str, "source": "feishu_bitable" }),
                };
                ToolResult {
                    success: true,
                    data: Some(json!({ "records": records })),
                    error: None,
                    artifacts: Some(vec![artifact]),
                    observation: format!("飞书多维表格记录如下：\n{md_str}"),
                    needs_auth: None,
                    continue_loop: None,
                }
            }
            Err(e) => feishu_err_to_result(e, "bitable:app:readonly"),
        }
    }
}

async fn do_query_bitable(token: &str, app_token: &str, table_id: &str, page_size: usize) -> anyhow::Result<Vec<serde_json::Value>> {
    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .get(format!(
            "https://open.feishu.cn/open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/records"
        ))
        .query(&[("page_size", &page_size.to_string())])
        .header("Authorization", format!("Bearer {token}"))
        .send().await?.error_for_status()?.json().await?;
    let code = resp.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code != 0 {
        return Err(anyhow::anyhow!(
            "飞书表格查询失败(code={}): {}",
            code,
            resp.get("msg").and_then(|v| v.as_str()).unwrap_or("unknown")
        ));
    }
    let records = resp
        .get("data").and_then(|d| d.get("items"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    // 简化：只取每条记录的 fields
    let simplified: Vec<serde_json::Value> = records
        .into_iter()
        .map(|r| r.get("fields").cloned().unwrap_or(serde_json::Value::Null))
        .collect();
    Ok(simplified)
}

// ============ 飞书日历事件列表工具 ============

pub struct FeishuCalendarListTool;

#[async_trait]
impl OfficeTool for FeishuCalendarListTool {
    fn name(&self) -> &str {
        "feishu_calendar_list"
    }

    fn description(&self) -> &str {
        "查询飞书日历的日程（事件）列表。可选传 start_time/end_time（Unix 秒时间戳）限定时间范围。"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "calendar_id": { "type": "string", "description": "日历 ID，可选，不传则查主日历" },
                "start_time": { "type": "string", "description": "开始时间 Unix 时间戳（秒），可选" },
                "end_time": { "type": "string", "description": "结束时间 Unix 时间戳（秒），可选" }
            },
            "required": []
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn produces_artifact(&self) -> bool {
        true
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let calendar_id = input.get("calendar_id").and_then(|v| v.as_str()).unwrap_or("primary");
        let start_time = input.get("start_time").and_then(|v| v.as_str()).unwrap_or("");
        let end_time = input.get("end_time").and_then(|v| v.as_str()).unwrap_or("");
        let identity = match resolve_feishu_access(ctx, Some("calendar:calendar:read")).await {
            Ok(id) => id,
            Err(needs) => return ToolResult::err_needs_auth(&needs.scope),
        };
        let token = match &identity {
            FeishuIdentity::User(t) | FeishuIdentity::App(t) => t.clone(),
        };
        match do_list_events(&token, calendar_id, start_time, end_time).await {
            Ok(events) => {
                let md_str = json_array_to_markdown(&events, "飞书日历日程");
                let artifact = ToolArtifact {
                    kind: "markdown".into(),
                    title: "飞书日历日程".into(),
                    content: json!({ "type": "markdown", "markdown": md_str, "source": "feishu_calendar" }),
                };
                ToolResult {
                    success: true,
                    data: Some(json!({ "events": events })),
                    error: None,
                    artifacts: Some(vec![artifact]),
                    observation: format!("飞书日历日程如下：\n{md_str}"),
                    needs_auth: None,
                    continue_loop: None,
                }
            }
            Err(e) => feishu_err_to_result(e, "calendar:calendar:read"),
        }
    }
}

async fn do_list_events(token: &str, calendar_id: &str, start_time: &str, end_time: &str) -> anyhow::Result<Vec<serde_json::Value>> {
    let client = reqwest::Client::new();
    let mut url = format!(
        "https://open.feishu.cn/open-apis/calendar/v4/calendars/{}/events",
        urlencoding::encode(calendar_id)
    );
    let mut first = true;
    if !start_time.is_empty() {
        url.push_str(&format!("{}start_time={}", if first { "?" } else { "&" }, start_time));
        first = false;
    }
    if !end_time.is_empty() {
        url.push_str(&format!("{}end_time={}", if first { "?" } else { "&" }, end_time));
    }
    let resp: serde_json::Value = client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .send().await?.error_for_status()?.json().await?;
    let code = resp.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code != 0 {
        return Err(anyhow::anyhow!(
            "飞书日历查询失败(code={}): {}",
            code,
            resp.get("msg").and_then(|v| v.as_str()).unwrap_or("unknown")
        ));
    }
    let events = resp
        .get("data").and_then(|d| d.get("items"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    // 简化：只取 summary + start_time + end_time + organizer
    let simplified: Vec<serde_json::Value> = events
        .into_iter()
        .map(|e| {
            json!({
                "summary": e.get("summary").cloned().unwrap_or(serde_json::Value::Null),
                "start_time": e.get("start_time").cloned().unwrap_or(serde_json::Value::Null),
                "end_time": e.get("end_time").cloned().unwrap_or(serde_json::Value::Null),
            })
        })
        .collect();
    Ok(simplified)
}

// ============ 飞书文档创建工具 ============

pub struct FeishuDocCreateTool;

#[async_trait]
impl OfficeTool for FeishuDocCreateTool {
    fn name(&self) -> &str {
        "feishu_doc_create"
    }

    fn description(&self) -> &str {
        "在飞书云文档中创建一篇新文档。输入文档标题和可选内容，返回新文档的 document_id。"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "文档标题" },
                "content": { "type": "string", "description": "文档正文内容（可选，Markdown）" }
            },
            "required": ["title"]
        })
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn produces_artifact(&self) -> bool {
        true
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let title = input.get("title").and_then(|v| v.as_str()).unwrap_or("").trim();
        let content = input.get("content").and_then(|v| v.as_str()).unwrap_or("").trim();
        if title.is_empty() {
            return ToolResult::err("title 不能为空");
        }
        let identity = match resolve_feishu_access(ctx, Some("docx:document:create")).await {
            Ok(id) => id,
            Err(needs) => return ToolResult::err_needs_auth(&needs.scope),
        };
        let token = match &identity {
            FeishuIdentity::User(t) | FeishuIdentity::App(t) => t.clone(),
        };
        match do_create_doc(&token, title, content).await {
            Ok(doc_id) => {
                let artifact = ToolArtifact {
                    kind: "markdown".into(),
                    title: format!("飞书文档已创建 · {title}"),
                    content: json!({ "type": "markdown", "markdown": format!("文档已创建：{doc_id}"), "source": "feishu_doc_create" }),
                };
                ToolResult {
                    success: true,
                    data: Some(json!({ "document_id": doc_id, "title": title })),
                    error: None,
                    artifacts: Some(vec![artifact]),
                    observation: format!("已创建飞书文档「{title}」，document_id={doc_id}"),
                    needs_auth: None,
                    continue_loop: None,
                }
            }
            Err(e) => feishu_err_to_result(e, "docx:document:create"),
        }
    }
}

async fn do_create_doc(token: &str, title: &str, content: &str) -> anyhow::Result<String> {
    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .post("https://open.feishu.cn/open-apis/docx/v1/documents")
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "title": title }))
        .send().await?.error_for_status()?.json().await?;
    let code = resp.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code != 0 {
        return Err(anyhow::anyhow!(
            "创建文档失败(code={}): {}",
            code,
            resp.get("msg").and_then(|v| v.as_str()).unwrap_or("unknown")
        ));
    }
    let doc_id = resp.get("data").and_then(|d| d.get("document")).and_then(|d| d.get("document_id")).and_then(|v| v.as_str()).unwrap_or("").to_string();
    if doc_id.is_empty() {
        return Err(anyhow::anyhow!("创建文档响应缺少 document_id"));
    }
    Ok(doc_id)
}

// ============ 飞书多维表格创建记录工具 ============

pub struct FeishuBitableCreateRecordTool;

#[async_trait]
impl OfficeTool for FeishuBitableCreateRecordTool {
    fn name(&self) -> &str {
        "feishu_bitable_create_record"
    }

    fn description(&self) -> &str {
        "向飞书多维表格（Bitable）新增一条记录。输入 app_token、table_id 和字段值（fields，JSON 对象）。"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "app_token": { "type": "string", "description": "多维表格 app_token" },
                "table_id": { "type": "string", "description": "数据表 table_id" },
                "fields": { "type": "object", "description": "字段值，如 {\"标题\": \"xxx\", \"状态\": \"进行中\"}" }
            },
            "required": ["app_token", "table_id", "fields"]
        })
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn produces_artifact(&self) -> bool {
        false
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let app_token = input.get("app_token").and_then(|v| v.as_str()).unwrap_or("").trim();
        let table_id = input.get("table_id").and_then(|v| v.as_str()).unwrap_or("").trim();
        let fields = input.get("fields").cloned().unwrap_or(serde_json::json!({}));
        if app_token.is_empty() || table_id.is_empty() {
            return ToolResult::err("app_token 和 table_id 不能为空");
        }
        let identity = match resolve_feishu_access(ctx, Some("bitable:app")).await {
            Ok(id) => id,
            Err(needs) => return ToolResult::err_needs_auth(&needs.scope),
        };
        let token = match &identity {
            FeishuIdentity::User(t) | FeishuIdentity::App(t) => t.clone(),
        };
        match do_create_record(&token, app_token, table_id, &fields).await {
            Ok(_) => ToolResult {
                success: true,
                data: Some(json!({ "created": true })),
                error: None,
                artifacts: None,
                observation: "已向飞书多维表格新增一条记录".into(),
                needs_auth: None,
                continue_loop: None,
            },
            Err(e) => feishu_err_to_result(e, "bitable:app"),
        }
    }
}

async fn do_create_record(token: &str, app_token: &str, table_id: &str, fields: &serde_json::Value) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .post(format!(
            "https://open.feishu.cn/open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/records"
        ))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "fields": fields }))
        .send().await?.error_for_status()?.json().await?;
    let code = resp.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code != 0 {
        return Err(anyhow::anyhow!(
            "新增记录失败(code={}): {}",
            code,
            resp.get("msg").and_then(|v| v.as_str()).unwrap_or("unknown")
        ));
    }
    Ok(())
}

// ============ 飞书日历创建日程工具 ============

pub struct FeishuCalendarCreateEventTool;

#[async_trait]
impl OfficeTool for FeishuCalendarCreateEventTool {
    fn name(&self) -> &str {
        "feishu_calendar_create_event"
    }

    fn description(&self) -> &str {
        "在飞书日历中创建一条日程。输入日程标题、开始时间、结束时间（Unix 秒时间戳），可选日历 ID。"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "summary": { "type": "string", "description": "日程标题" },
                "start_time": { "type": "string", "description": "开始时间 Unix 秒时间戳" },
                "end_time": { "type": "string", "description": "结束时间 Unix 秒时间戳" },
                "calendar_id": { "type": "string", "description": "日历 ID，可选，默认主日历" }
            },
            "required": ["summary", "start_time", "end_time"]
        })
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn produces_artifact(&self) -> bool {
        false
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let summary = input.get("summary").and_then(|v| v.as_str()).unwrap_or("").trim();
        let start_time = input.get("start_time").and_then(|v| v.as_str()).unwrap_or("");
        let end_time = input.get("end_time").and_then(|v| v.as_str()).unwrap_or("");
        let calendar_id = input.get("calendar_id").and_then(|v| v.as_str()).unwrap_or("primary");
        if summary.is_empty() || start_time.is_empty() || end_time.is_empty() {
            return ToolResult::err("summary、start_time、end_time 不能为空");
        }
        let identity = match resolve_feishu_access(ctx, Some("calendar:calendar.event:create")).await {
            Ok(id) => id,
            Err(needs) => return ToolResult::err_needs_auth(&needs.scope),
        };
        let token = match &identity {
            FeishuIdentity::User(t) | FeishuIdentity::App(t) => t.clone(),
        };
        match do_create_event(&token, calendar_id, summary, start_time, end_time).await {
            Ok(_) => ToolResult {
                success: true,
                data: Some(json!({ "created": true, "summary": summary })),
                error: None,
                artifacts: None,
                observation: format!("已创建日程「{summary}」"),
                needs_auth: None,
                continue_loop: None,
            },
            Err(e) => feishu_err_to_result(e, "calendar:calendar.event:create"),
        }
    }
}

async fn do_create_event(token: &str, calendar_id: &str, summary: &str, start_time: &str, end_time: &str) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .post(format!(
            "https://open.feishu.cn/open-apis/calendar/v4/calendars/{}/events",
            urlencoding::encode(calendar_id)
        ))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "summary": summary,
            "start_time": { "timestamp": start_time },
            "end_time": { "timestamp": end_time },
        }))
        .send().await?.error_for_status()?.json().await?;
    let code = resp.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code != 0 {
        return Err(anyhow::anyhow!(
            "创建日程失败(code={}): {}",
            code,
            resp.get("msg").and_then(|v| v.as_str()).unwrap_or("unknown")
        ));
    }
    Ok(())
}

// ============ 飞书云盘列表工具 ============

pub struct FeishuDriveListTool;

#[async_trait]
impl OfficeTool for FeishuDriveListTool {
    fn name(&self) -> &str {
        "feishu_drive_list"
    }

    fn description(&self) -> &str {
        "列出飞书云盘的文件列表。可选传入 folder_token 查看指定文件夹；不传则列出根目录/最近文件。"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "folder_token": { "type": "string", "description": "文件夹 token，可选" }
            },
            "required": []
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn produces_artifact(&self) -> bool {
        true
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let folder_token = input.get("folder_token").and_then(|v| v.as_str()).unwrap_or("");
        let identity = match resolve_feishu_access(ctx, Some("drive:drive:readonly")).await {
            Ok(id) => id,
            Err(needs) => return ToolResult::err_needs_auth(&needs.scope),
        };
        let token = match &identity {
            FeishuIdentity::User(t) | FeishuIdentity::App(t) => t.clone(),
        };
        match do_list_drive(&token, folder_token).await {
            Ok(files) => {
                let md_str = json_array_to_markdown(&files, "飞书云盘文件列表");
                let artifact = ToolArtifact {
                    kind: "markdown".into(),
                    title: "飞书云盘文件列表".into(),
                    content: json!({ "type": "markdown", "markdown": md_str, "source": "feishu_drive" }),
                };
                ToolResult {
                    success: true,
                    data: Some(json!({ "files": files })),
                    error: None,
                    artifacts: Some(vec![artifact]),
                    observation: format!("飞书云盘文件列表：\n{md_str}"),
                    needs_auth: None,
                    continue_loop: None,
                }
            }
            Err(e) => feishu_err_to_result(e, "drive:drive:readonly"),
        }
    }
}

async fn do_list_drive(token: &str, folder_token: &str) -> anyhow::Result<Vec<serde_json::Value>> {
    let client = reqwest::Client::new();
    let mut url = "https://open.feishu.cn/open-apis/drive/v1/files".to_string();
    if !folder_token.is_empty() {
        url.push_str(&format!("?folder_token={}", folder_token));
    }
    let resp: serde_json::Value = client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .send().await?.error_for_status()?.json().await?;
    let code = resp.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code != 0 {
        return Err(anyhow::anyhow!(
            "列云盘文件失败(code={}): {}",
            code,
            resp.get("msg").and_then(|v| v.as_str()).unwrap_or("unknown")
        ));
    }
    let files = resp
        .get("data").and_then(|d| d.get("files"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let simplified: Vec<serde_json::Value> = files
        .into_iter()
        .map(|f| {
            json!({
                "token": f.get("token").cloned().unwrap_or(serde_json::Value::Null),
                "name": f.get("name").cloned().unwrap_or(serde_json::Value::Null),
                "type": f.get("type").cloned().unwrap_or(serde_json::Value::Null),
            })
        })
        .collect();
    Ok(simplified)
}

// ============ 飞书知识库搜索工具 ============

pub struct FeishuWikiSearchTool;

#[async_trait]
impl OfficeTool for FeishuWikiSearchTool {
    fn name(&self) -> &str {
        "feishu_wiki_search"
    }

    fn description(&self) -> &str {
        "搜索飞书知识库（Wiki）节点。输入关键词，返回匹配的知识库节点（标题 + 链接 + 摘要）。"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "搜索关键词" }
            },
            "required": ["query"]
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn produces_artifact(&self) -> bool {
        true
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let query = input.get("query").and_then(|v| v.as_str()).unwrap_or("").trim();
        if query.is_empty() {
            return ToolResult::err("query 不能为空");
        }
        let identity = match resolve_feishu_access(ctx, Some("wiki:wiki:readonly")).await {
            Ok(id) => id,
            Err(needs) => return ToolResult::err_needs_auth(&needs.scope),
        };
        let token = match &identity {
            FeishuIdentity::User(t) | FeishuIdentity::App(t) => t.clone(),
        };
        match do_search_wiki(&token, query).await {
            Ok(nodes) => {
                let md_str = json_array_to_markdown(&nodes, "飞书知识库搜索结果");
                let artifact = ToolArtifact {
                    kind: "markdown".into(),
                    title: format!("知识库搜索 · {query}"),
                    content: json!({ "type": "markdown", "markdown": md_str, "source": "feishu_wiki" }),
                };
                ToolResult {
                    success: true,
                    data: Some(json!({ "nodes": nodes })),
                    error: None,
                    artifacts: Some(vec![artifact]),
                    observation: format!("知识库搜索结果：\n{md_str}"),
                    needs_auth: None,
                    continue_loop: None,
                }
            }
            Err(e) => feishu_err_to_result(e, "wiki:wiki:readonly"),
        }
    }
}

async fn do_search_wiki(token: &str, query: &str) -> anyhow::Result<Vec<serde_json::Value>> {
    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .get("https://open.feishu.cn/open-apis/wiki/v2/search")
        .header("Authorization", format!("Bearer {token}"))
        .query(&[("query", query)])
        .send().await?.error_for_status()?.json().await?;
    let code = resp.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code != 0 {
        return Err(anyhow::anyhow!(
            "搜索知识库失败(code={}): {}",
            code,
            resp.get("msg").and_then(|v| v.as_str()).unwrap_or("unknown")
        ));
    }
    let nodes = resp
        .get("data").and_then(|d| d.get("items"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let simplified: Vec<serde_json::Value> = nodes
        .into_iter()
        .map(|n| {
            json!({
                "title": n.pointer("/title").cloned().unwrap_or(serde_json::Value::Null),
                "url": n.pointer("/url").cloned().unwrap_or(serde_json::Value::Null),
                "node_token": n.pointer("/node_token").cloned().unwrap_or(serde_json::Value::Null),
            })
        })
        .collect();
    Ok(simplified)
}

use async_trait::async_trait;
use serde_json::json;

use crate::agent::tool::{OfficeTool, ToolArtifact, ToolContext, ToolResult};
use crate::models::NasConfig;

/// 读取当前用户的 NAS 挂载凭据（按 user_id 隔离，多租户互不冲突）
async fn get_nas_config(ctx: &ToolContext) -> anyhow::Result<NasConfig> {
    let pool = crate::state::db_pool();
    let settings = crate::db::settings_repo::find_by_user(&pool, &ctx.user_id)
        .await?
        .unwrap_or_else(crate::routes::settings::default_settings);
    let cfg = settings.nas_config;
    if !cfg.enabled || cfg.base_url.is_empty() {
        return Err(anyhow::anyhow!(
            "尚未配置 NAS 挂载，请先在「设置 → 数据源」中填写懒猫微服 WebDAV 地址与凭据"
        ));
    }
    if cfg.username.is_empty() {
        return Err(anyhow::anyhow!("NAS 挂载缺少用户名"));
    }
    Ok(cfg)
}

/// 构造带 Basic Auth 的 reqwest client
fn dav_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default()
}

/// 拼接 WebDAV 路径（去掉末尾/开头的多余斜杠）
fn join_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    if path.is_empty() {
        base.to_string()
    } else {
        format!("{}/{}", base, path.trim_start_matches('/'))
    }
}

/// PROPFIND 列目录（Depth: 1），返回 (名称, 是否目录, 大小, 最后修改)
async fn dav_list(
    cfg: &NasConfig,
    path: &str,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let client = dav_client();
    let url = join_url(&cfg.base_url, path);
    let resp = client
        .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &url)
        .basic_auth(&cfg.username, Some(&cfg.password))
        .header("Depth", "1")
        .header("Content-Type", "application/xml")
        .body(
            r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:displayname/>
    <d:resourcetype/>
    <d:getcontentlength/>
    <d:getlastmodified/>
  </d:prop>
</d:propfind>"#,
        )
        .send()
        .await?;

    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        return Err(anyhow::anyhow!("WebDAV 请求失败（HTTP {status}）: {}", body.chars().take(300).collect::<String>()));
    }

    parse_propfind_response(&body)
}

/// 解析 PROPFIND 的 multistatus XML
fn parse_propfind_response(body: &str) -> anyhow::Result<Vec<serde_json::Value>> {
    let mut items = Vec::new();
    // 用简易 XML 解析：按 <d:response> 分段
    let doc = roxmltree::Document::parse(body).map_err(|e| anyhow::anyhow!("解析 WebDAV 响应失败: {e}"))?;

    for response in doc.descendants().filter(|n| n.has_tag_name("response")) {
        let mut href = String::new();
        let mut displayname = String::new();
        let mut is_collection = false;
        let mut size = 0i64;
        let mut lastmod = String::new();

        for node in response.descendants() {
            if node.has_tag_name("href") {
                if let Some(t) = node.text() {
                    href = t.to_string();
                }
            }
            if node.has_tag_name("displayname") {
                if let Some(t) = node.text() {
                    displayname = t.to_string();
                }
            }
            if node.has_tag_name("collection") {
                is_collection = true;
            }
            if node.has_tag_name("getcontentlength") {
                if let Some(t) = node.text() {
                    size = t.trim().parse().unwrap_or(0);
                }
            }
            if node.has_tag_name("getlastmodified") {
                if let Some(t) = node.text() {
                    lastmod = t.to_string();
                }
            }
        }

        if href.is_empty() || displayname.is_empty() {
            continue;
        }
        // 跳过当前目录本身
        let is_self = href.trim_end_matches('/') == displayname.trim_end_matches('/')
            || displayname == "."
            || displayname.is_empty();
        if is_self {
            continue;
        }

        items.push(json!({
            "name": displayname,
            "href": href,
            "is_dir": is_collection,
            "size": size,
            "modified": lastmod,
        }));
    }

    if items.is_empty() && body.contains("multistatus") {
        // 空目录（只有自身）也是合法的，返回空数组
        return Ok(items);
    }
    Ok(items)
}

/// GET 读取文件内容
async fn dav_read(cfg: &NasConfig, path: &str) -> anyhow::Result<(String, String)> {
    let client = dav_client();
    let url = join_url(&cfg.base_url, path);
    let resp = client
        .get(&url)
        .basic_auth(&cfg.username, Some(&cfg.password))
        .send()
        .await?;
    let status = resp.status();
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let bytes = resp.bytes().await?;
    if !status.is_success() {
        return Err(anyhow::anyhow!("读取文件失败（HTTP {status}）"));
    }
    let text = String::from_utf8_lossy(&bytes).to_string();
    Ok((text, content_type))
}

/// PUT 写入文件
async fn dav_write(cfg: &NasConfig, path: &str, content: &str) -> anyhow::Result<()> {
    let client = dav_client();
    let url = join_url(&cfg.base_url, path);
    let resp = client
        .put(&url)
        .basic_auth(&cfg.username, Some(&cfg.password))
        .body(content.to_string())
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("写入文件失败（HTTP {status}）: {}", body.chars().take(200).collect::<String>()));
    }
    Ok(())
}

/// MKCOL 建目录
async fn dav_mkdir(cfg: &NasConfig, path: &str) -> anyhow::Result<()> {
    let client = dav_client();
    let url = join_url(&cfg.base_url, path);
    let resp = client
        .request(reqwest::Method::from_bytes(b"MKCOL").unwrap(), &url)
        .basic_auth(&cfg.username, Some(&cfg.password))
        .send()
        .await?;
    let status = resp.status();
    // 201 Created / 405 已存在都算可接受
    if !status.is_success() && status.as_u16() != 405 {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("创建目录失败（HTTP {status}）: {}", body.chars().take(200).collect::<String>()));
    }
    Ok(())
}

/// 把 NAS 列表结果序列化为 markdown 表格文本
fn files_to_markdown(files: &[serde_json::Value]) -> String {
    if files.is_empty() {
        return "（空目录）".to_string();
    }
    let mut lines = vec!["| 名称 | 类型 | 大小 | 修改时间 |".to_string(), "| --- | --- | --- | --- |".to_string()];
    for f in files {
        let name = f.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let is_dir = f.get("is_dir").and_then(|v| v.as_bool()).unwrap_or(false);
        let kind = if is_dir { "📁 目录" } else { "📄 文件" };
        let size = f.get("size").and_then(|v| v.as_i64()).unwrap_or(0);
        let size_str = if is_dir { "-".to_string() } else { format!("{} B", size) };
        let modified = f.get("modified").and_then(|v| v.as_str()).unwrap_or("");
        lines.push(format!("| {} | {} | {} | {} |", name, kind, size_str, modified));
    }
    lines.join("\n")
}

/// NAS 列出目录工具
pub struct NasListTool;

#[async_trait]
impl OfficeTool for NasListTool {
    fn name(&self) -> &str {
        "nas_list"
    }

    fn description(&self) -> &str {
        "列出懒猫微服 NAS（WebDAV）指定目录的文件与子目录。path 为空时列根目录。凭据按用户隔离，只访问当前用户自己挂载的 NAS。"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "NAS 上的目录路径（相对挂载根，如 /docs 或空字符串表示根目录）" }
            },
            "required": []
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let cfg = match get_nas_config(ctx).await {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("{e}")),
        };
        match dav_list(&cfg, path).await {
            Ok(files) => {
                let md = files_to_markdown(&files);
                let artifact = ToolArtifact {
                    kind: "markdown".into(),
                    title: format!("NAS 目录列表{}", if path.is_empty() { "（根）".to_string() } else { format!("：{path}") }),
                    content: json!({ "type": "markdown", "markdown": md, "source": "nas" }),
                };
                ToolResult {
                    success: true,
                    data: Some(json!({ "files": files, "path": path })),
                    error: None,
                    artifacts: Some(vec![artifact]),
                    observation: format!("NAS 目录列表（{}）：\n{md}", if path.is_empty() { "/" } else { path }),
                    needs_auth: None,
                    continue_loop: None,
                }
            }
            Err(e) => ToolResult::err(format!("列 NAS 目录失败: {e}")),
        }
    }
}

/// NAS 读取文件工具
pub struct NasReadTool;

#[async_trait]
impl OfficeTool for NasReadTool {
    fn name(&self) -> &str {
        "nas_read"
    }

    fn description(&self) -> &str {
        "读取懒猫微服 NAS（WebDAV）上指定文件的内容（文本类文件）。凭据按用户隔离。"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "NAS 上的文件路径（相对挂载根，如 /docs/方案.md）" }
            },
            "required": ["path"]
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
        if path.is_empty() {
            return ToolResult::err("缺少 path 参数");
        }
        let cfg = match get_nas_config(ctx).await {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("{e}")),
        };
        match dav_read(&cfg, path).await {
            Ok((text, _ctype)) => {
                let preview = text.chars().take(20000).collect::<String>();
                let truncated = text.chars().count() > 20000;
                let artifact = ToolArtifact {
                    kind: "markdown".into(),
                    title: format!("NAS 文件：{path}"),
                    content: json!({ "type": "markdown", "markdown": preview, "source": "nas" }),
                };
                ToolResult {
                    success: true,
                    data: Some(json!({ "path": path, "truncated": truncated })),
                    error: None,
                    artifacts: Some(vec![artifact]),
                    observation: format!("NAS 文件内容（{path}）：\n{preview}{}", if truncated { "\n…（内容过长已截断）" } else { "" }),
                    needs_auth: None,
                    continue_loop: None,
                }
            }
            Err(e) => ToolResult::err(format!("读 NAS 文件失败: {e}")),
        }
    }
}

/// NAS 写入文件工具
pub struct NasWriteTool;

#[async_trait]
impl OfficeTool for NasWriteTool {
    fn name(&self) -> &str {
        "nas_write"
    }

    fn description(&self) -> &str {
        "把内容写入懒猫微服 NAS（WebDAV）上的指定文件（覆盖写）。凭据按用户隔离。"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "NAS 上的文件路径（相对挂载根，如 /docs/纪要.md）" },
                "content": { "type": "string", "description": "要写入的文件内容" }
            },
            "required": ["path", "content"]
        })
    }

    fn is_read_only(&self) -> bool {
        false
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let content = input.get("content").and_then(|v| v.as_str()).unwrap_or("");
        if path.is_empty() {
            return ToolResult::err("缺少 path 参数");
        }
        let cfg = match get_nas_config(ctx).await {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("{e}")),
        };
        match dav_write(&cfg, path, content).await {
            Ok(()) => ToolResult {
                success: true,
                data: Some(json!({ "path": path, "written": content.chars().count() })),
                error: None,
                artifacts: None,
                observation: format!("已写入 NAS 文件：{path}（{} 字符）", content.chars().count()),
                needs_auth: None,
                continue_loop: None,
            },
            Err(e) => ToolResult::err(format!("写 NAS 文件失败: {e}")),
        }
    }
}

/// NAS 建目录工具
pub struct NasMkdirTool;

#[async_trait]
impl OfficeTool for NasMkdirTool {
    fn name(&self) -> &str {
        "nas_mkdir"
    }

    fn description(&self) -> &str {
        "在懒猫微服 NAS（WebDAV）上创建目录。凭据按用户隔离。"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "要创建的目录路径（相对挂载根，如 /docs/2026）" }
            },
            "required": ["path"]
        })
    }

    fn is_read_only(&self) -> bool {
        false
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
        if path.is_empty() {
            return ToolResult::err("缺少 path 参数");
        }
        let cfg = match get_nas_config(ctx).await {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("{e}")),
        };
        match dav_mkdir(&cfg, path).await {
            Ok(()) => ToolResult {
                success: true,
                data: Some(json!({ "path": path })),
                error: None,
                artifacts: None,
                observation: format!("已在 NAS 创建目录：{path}"),
                needs_auth: None,
                continue_loop: None,
            },
            Err(e) => ToolResult::err(format!("建 NAS 目录失败: {e}")),
        }
    }
}

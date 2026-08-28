use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::auth::middleware::AuthUser;
use crate::db::settings_repo;
use crate::error::AppError;
use crate::models::{AppSettings, BasicSettings, LlmProfileConfig, McpServerConfig, NasConfig};
use crate::state;
use crate::agent::tools::agnes_media::build_endpoint;
use eventsource_stream::Eventsource;
use futures::StreamExt;

pub fn router() -> Router {
    Router::new()
        .route("/api/settings", get(get_settings).put(save_settings))
        .route("/api/settings/mcp/test", post(test_mcp_service))
        .route("/api/settings/fetch-models", post(fetch_models))
        .route("/api/settings/nas/test", post(test_nas))
        .route("/api/settings/media/test", post(test_media_model))
        .route("/api/settings/llm/test", post(test_llm_capability))
}

#[derive(Debug, Deserialize)]
struct FetchModelsReq {
    base_url: String,
    #[serde(default)]
    api_key: String,
}

#[derive(Debug, Deserialize)]
struct TestMediaReq {
    base_url: String,
    #[serde(default)]
    api_key: String,
}

#[derive(Debug, Deserialize)]
struct TestLlmReq {
    /// 检测类型：text（推理）/ image（生图）/ video（生视频）
    kind: String,
    base_url: String,
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    model: String,
}

pub fn default_settings() -> AppSettings {
    let cfg = crate::config::config();
    let models = configured_models(cfg);
    let default_profile = LlmProfileConfig {
        id: "default".into(),
        name: "默认模型服务".into(),
        base_url: cfg.llm_base_url.clone(),
        api_keys: if cfg.llm_api_key.trim().is_empty() {
            vec![]
        } else {
            vec![cfg.llm_api_key.clone()]
        },
        models,
        default_model: cfg.llm_model.clone(),
        api_key: None,
        has_api_key: !cfg.llm_api_key.is_empty(),
    };

    AppSettings {
        llm_profiles: vec![default_profile.clone()],
        active_profile_id: default_profile.id.clone(),
        default_model: default_profile.default_model.clone(),
        active_model: default_profile.default_model.clone(),
        basic: BasicSettings {
            app_name: cfg.app_name.clone(),
            workspace_title: "智能办公助手".into(),
            brand_tagline: "打开即用，专注办公创作".into(),
            default_theme: "default".into(),
        },
        mcp_servers: builtin_mcp_servers(),
        search_providers: crate::models::SearchProvidersConfig {
            provider: "auto".into(),
            ..Default::default()
        },
        feishu_token: Default::default(),
        nas_config: Default::default(),
        image_profile: crate::models::MediaProfileConfig {
            base_url: cfg.llm_image_base_url.clone(),
            api_keys: cfg.llm_image_api_keys.clone(),
            api_key: cfg.llm_image_api_key.clone(),
            model: cfg.llm_image_model.clone(),
        },
        video_profile: crate::models::MediaProfileConfig {
            base_url: cfg.llm_video_base_url.clone(),
            api_keys: cfg.llm_video_api_keys.clone(),
            api_key: cfg.llm_video_api_key.clone(),
            model: cfg.llm_video_model.clone(),
        },
        updated_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn configured_models(cfg: &crate::config::Config) -> Vec<String> {
    let mut models = Vec::new();
    for model in cfg
        .llm_text_models
        .iter()
        .chain(cfg.llm_image_models.iter())
        .chain(cfg.llm_video_models.iter())
    {
        let model = model.trim();
        if !model.is_empty() && !models.iter().any(|item| item == model) {
            models.push(model.to_string());
        }
    }
    models
}

fn builtin_mcp_servers() -> Vec<McpServerConfig> {
    vec![]
}

pub fn normalize_settings(mut settings: AppSettings) -> Result<AppSettings, AppError> {
    if settings.llm_profiles.is_empty() {
        return Err(AppError::BadRequest("至少保留一个模型服务配置".into()));
    }

    for profile in &mut settings.llm_profiles {
        if profile.id.trim().is_empty() {
            profile.id = uuid::Uuid::new_v4().to_string();
        }
        if profile.name.trim().is_empty() {
            profile.name = "未命名模型服务".into();
        }
        let mut api_keys: Vec<String> = Vec::new();
        for api_key in &profile.api_keys {
            let api_key = api_key.trim().to_string();
            if !api_key.is_empty() && !api_keys.iter().any(|item| item == &api_key) {
                api_keys.push(api_key);
            }
        }
        if let Some(api_key) = profile.api_key.take() {
            let api_key = api_key.trim().to_string();
            if !api_key.is_empty() && !api_keys.iter().any(|item| item == &api_key) {
                api_keys.push(api_key);
            }
        }
        profile.api_keys = api_keys;
        profile.models = profile
            .models
            .iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect();
        if profile.models.is_empty() {
            return Err(AppError::BadRequest(format!(
                "模型服务「{}」至少需要一个模型",
                profile.name
            )));
        }
        if profile.default_model.trim().is_empty()
            || !profile
                .models
                .iter()
                .any(|item| item == &profile.default_model)
        {
            profile.default_model = profile.models[0].clone();
        }
        profile.has_api_key = !profile.api_keys.is_empty();
    }

    if !settings
        .llm_profiles
        .iter()
        .any(|profile| profile.id == settings.active_profile_id)
    {
        settings.active_profile_id = settings.llm_profiles[0].id.clone();
    }

    let active_profile = settings
        .llm_profiles
        .iter()
        .find(|profile| profile.id == settings.active_profile_id)
        .cloned()
        .unwrap_or_else(|| settings.llm_profiles[0].clone());

    if settings.default_model.trim().is_empty()
        || !active_profile
            .models
            .iter()
            .any(|item| item == &settings.default_model)
    {
        settings.default_model = active_profile.default_model.clone();
    }
    if settings.active_model.trim().is_empty()
        || !active_profile
            .models
            .iter()
            .any(|item| item == &settings.active_model)
    {
        settings.active_model = settings.default_model.clone();
    }

    if settings.basic.app_name.trim().is_empty() {
        settings.basic.app_name = crate::config::config().app_name.clone();
    }
    if settings.basic.workspace_title.trim().is_empty() {
        settings.basic.workspace_title = "智能办公助手".into();
    }
    if settings.basic.brand_tagline.trim().is_empty() {
        settings.basic.brand_tagline = "打开即用，专注办公创作".into();
    }
    if settings.basic.default_theme.trim().is_empty() {
        settings.basic.default_theme = "default".into();
    }

    for builtin in builtin_mcp_servers() {
        if !settings
            .mcp_servers
            .iter()
            .any(|server| server.id == builtin.id || server.endpoint == builtin.endpoint)
        {
            settings.mcp_servers.push(builtin);
        }
    }

    settings.updated_at = chrono::Utc::now().to_rfc3339();
    Ok(settings)
}

pub fn import_startup_llm_profile(settings: &mut AppSettings) {
    let cfg = crate::config::config();
    let base_url = cfg.llm_base_url.trim();
    let model = cfg.llm_model.trim();
    if base_url.is_empty() || model.is_empty() {
        return;
    }

    let startup_keys = if cfg.llm_api_key.trim().is_empty() {
        vec![]
    } else {
        vec![cfg.llm_api_key.trim().to_string()]
    };

    if let Some(profile) = settings.llm_profiles.iter_mut().find(|profile| {
        profile.id == "startup-env"
            || profile.base_url.trim_end_matches('/') == base_url.trim_end_matches('/')
    }) {
        if profile.id == "startup-env" {
            profile.name = "启动配置模型服务".into();
            profile.base_url = base_url.to_string();
        }
        if !profile.models.iter().any(|item| item == model) {
            profile.models.insert(0, model.to_string());
        }
        for configured in configured_models(cfg) {
            if !profile.models.iter().any(|item| item == &configured) {
                profile.models.push(configured);
            }
        }
        for api_key in startup_keys {
            if !profile.api_keys.iter().any(|item| item == &api_key) {
                profile.api_keys.push(api_key);
            }
        }
        if profile.default_model.trim().is_empty() {
            profile.default_model = model.to_string();
        }
        return;
    }

    let models = configured_models(cfg);

    settings.llm_profiles.push(LlmProfileConfig {
        id: "startup-env".into(),
        name: "启动配置模型服务".into(),
        base_url: base_url.to_string(),
        api_keys: startup_keys,
        models,
        default_model: model.to_string(),
        api_key: None,
        has_api_key: !cfg.llm_api_key.trim().is_empty(),
    });
}

async fn get_settings(user: AuthUser) -> Result<Json<AppSettings>, AppError> {
    let pool = state::db_pool();
    let settings = settings_repo::find_by_user(&pool, &user.0.id).await?.unwrap_or_else(default_settings);
    Ok(Json(normalize_settings(settings)?))
}

async fn save_settings(
    user: AuthUser,
    Json(payload): Json<AppSettings>,
) -> Result<Json<AppSettings>, AppError> {
    let pool = state::db_pool();
    // 防御性合并：若前端漏传飞书 token（前端 types 未声明该字段），保留已存的 token，避免覆盖丢失
    let mut payload = payload;
    if payload.feishu_token.user_access_token.is_empty() && payload.feishu_token.refresh_token.is_empty() {
        if let Ok(Some(existing)) = settings_repo::find_by_user(&pool, &user.0.id).await {
            if !existing.feishu_token.user_access_token.is_empty() || !existing.feishu_token.refresh_token.is_empty() {
                payload.feishu_token = existing.feishu_token;
            }
        }
    }
    let normalized = normalize_settings(payload)?;
    let saved = settings_repo::save_for_user(&pool, &user.0.id, &normalized).await?;
    Ok(Json(saved))
}

async fn test_mcp_service(
    _user: AuthUser,
    Json(payload): Json<McpServerConfig>,
) -> Result<Json<serde_json::Value>, AppError> {
    if payload.transport == "sse" {
        return test_sse_mcp_service(&payload).await;
    }

    if payload.transport != "http" {
        return Ok(Json(json!({
            "ok": false,
            "message": format!("当前仅支持测试 HTTP 类型 MCP 服务，暂不支持 {}", payload.transport),
            "tools": []
        })));
    }

    let endpoint = payload.endpoint.trim().trim_end_matches('/').to_string();
    if endpoint.is_empty() {
        return Err(AppError::BadRequest("MCP 服务地址不能为空".into()));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("构建 MCP 测试客户端失败: {e}")))?;

    let initialize_resp = client
        .post(&endpoint)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "WaLiOffice",
                    "version": "0.2.0"
                }
            }
        }))
        .send()
        .await
        .map_err(|e| AppError::BadRequest(format!("连接 MCP 服务失败: {e}")))?;

    let status = initialize_resp.status();
    let init_text = initialize_resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Ok(Json(json!({
            "ok": false,
            "message": format!("初始化失败: HTTP {} {}", status.as_u16(), init_text),
            "tools": []
        })));
    }

    let _ = client
        .post(&endpoint)
        .header("Content-Type", "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }))
        .send()
        .await;

    let tools_resp = client
        .post(&endpoint)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }))
        .send()
        .await
        .map_err(|e| AppError::BadRequest(format!("读取 MCP 工具列表失败: {e}")))?;

    let tools_status = tools_resp.status();
    let tools_text = tools_resp.text().await.unwrap_or_default();
    if !tools_status.is_success() {
        return Ok(Json(json!({
            "ok": false,
            "message": format!("获取工具列表失败: HTTP {} {}", tools_status.as_u16(), tools_text),
            "tools": []
        })));
    }

    let payload_value: serde_json::Value = serde_json::from_str(&tools_text)
        .map_err(|e| AppError::BadRequest(format!("解析 MCP 工具列表失败: {e}")))?;
    let tools = payload_value
        .get("result")
        .and_then(|result| result.get("tools"))
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(Json(json!({
        "ok": true,
        "message": format!("MCP 服务连接成功，共发现 {} 个工具", tools.len()),
        "tools": tools
    })))
}

async fn test_sse_mcp_service(
    payload: &McpServerConfig,
) -> Result<Json<serde_json::Value>, AppError> {
    let endpoint = payload.endpoint.trim().to_string();
    if endpoint.is_empty() {
        return Err(AppError::BadRequest("MCP SSE 地址不能为空".into()));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("构建 MCP SSE 客户端失败: {e}")))?;

    let sse_resp = client
        .get(&endpoint)
        .header("Accept", "text/event-stream")
        .send()
        .await
        .map_err(|e| AppError::BadRequest(format!("连接 MCP SSE 服务失败: {e}")))?;

    if !sse_resp.status().is_success() {
        let status = sse_resp.status();
        let body = sse_resp.text().await.unwrap_or_default();
        return Ok(Json(json!({
            "ok": false,
            "message": format!("连接 MCP SSE 服务失败: HTTP {} {}", status.as_u16(), body),
            "tools": []
        })));
    }

    let mut stream = sse_resp.bytes_stream().eventsource();
    let message_endpoint = loop {
        let Some(event) = stream.next().await else {
            return Ok(Json(json!({
                "ok": false,
                "message": "MCP SSE 服务未返回 endpoint 事件",
                "tools": []
            })));
        };

        match event {
            Ok(ev) if ev.event == "endpoint" && !ev.data.trim().is_empty() => {
                break if ev.data.starts_with("http://") || ev.data.starts_with("https://") {
                    ev.data
                } else {
                    return Ok(Json(json!({
                        "ok": false,
                        "message": "MCP SSE 服务返回了非绝对 endpoint",
                        "tools": []
                    })));
                };
            }
            Ok(_) => {}
            Err(err) => {
                return Ok(Json(json!({
                    "ok": false,
                    "message": format!("解析 MCP SSE 事件失败: {err}"),
                    "tools": []
                })));
            }
        }
    };

    let request_headers = [
        ("Content-Type", "application/json"),
        ("Accept", "application/json, text/event-stream"),
    ];

    let initialize_resp = client
        .post(&message_endpoint)
        .headers(request_headers.iter().fold(
            reqwest::header::HeaderMap::new(),
            |mut headers, (key, value)| {
                headers.insert(*key, reqwest::header::HeaderValue::from_static(value));
                headers
            },
        ))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "WaLiOffice",
                    "version": "0.2.0"
                }
            }
        }))
        .send()
        .await
        .map_err(|e| AppError::BadRequest(format!("发送 MCP initialize 失败: {e}")))?;

    if !initialize_resp.status().is_success() {
        return Ok(Json(json!({
            "ok": false,
            "message": format!("MCP initialize 失败: HTTP {}", initialize_resp.status().as_u16()),
            "tools": []
        })));
    }

    let _ = client
        .post(&message_endpoint)
        .header("Content-Type", "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }))
        .send()
        .await;

    let tools_resp = client
        .post(&message_endpoint)
        .headers(request_headers.iter().fold(
            reqwest::header::HeaderMap::new(),
            |mut headers, (key, value)| {
                headers.insert(*key, reqwest::header::HeaderValue::from_static(value));
                headers
            },
        ))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }))
        .send()
        .await
        .map_err(|e| AppError::BadRequest(format!("请求 MCP 工具列表失败: {e}")))?;

    if !tools_resp.status().is_success() {
        return Ok(Json(json!({
            "ok": false,
            "message": format!("MCP tools/list 失败: HTTP {}", tools_resp.status().as_u16()),
            "tools": []
        })));
    }

    let tools = loop {
        let Some(event) = stream.next().await else {
            return Ok(Json(json!({
                "ok": false,
                "message": "MCP SSE 未返回工具列表结果",
                "tools": []
            })));
        };

        match event {
            Ok(ev) if ev.event == "message" => {
                let payload_value: serde_json::Value = match serde_json::from_str(&ev.data) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if payload_value.get("id").and_then(|v| v.as_i64()) == Some(2) {
                    break payload_value
                        .get("result")
                        .and_then(|result| result.get("tools"))
                        .and_then(|value| value.as_array())
                        .cloned()
                        .unwrap_or_default();
                }
            }
            Ok(_) => {}
            Err(err) => {
                return Ok(Json(json!({
                    "ok": false,
                    "message": format!("读取 MCP SSE 工具列表结果失败: {err}"),
                    "tools": []
                })));
            }
        }
    };

    Ok(Json(json!({
        "ok": true,
        "message": format!("MCP SSE 服务连接成功，共发现 {} 个工具", tools.len()),
        "tools": tools
    })))
}

/// 拉取 LLM 服务的真实模型列表（OpenAI 兼容 /models 接口）
async fn fetch_models(
    _user: AuthUser,
    Json(req): Json<FetchModelsReq>,
) -> Result<Json<serde_json::Value>, AppError> {
    let base_url = req.base_url.trim().trim_end_matches('/').to_string();
    if base_url.is_empty() {
        return Err(AppError::BadRequest("Base URL 不能为空".into()));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("构建 HTTP 客户端失败: {e}")))?;

    // 兼容不同的 /models 路径：先试 {base_url}/models，再试 {base_url}/v1/models
    let endpoints = vec![
        format!("{base_url}/models"),
        format!("{base_url}/v1/models"),
    ];

    let mut last_err: Option<String> = None;
    let mut models: Vec<String> = Vec::new();

    for endpoint in endpoints {
        let mut builder = client.get(&endpoint);
        if !req.api_key.trim().is_empty() {
            builder = builder.header("Authorization", format!("Bearer {}", req.api_key.trim()));
        }
        match builder.send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    let json: serde_json::Value = resp
                        .json()
                        .await
                        .unwrap_or(serde_json::Value::Null);
                    // OpenAI 兼容格式：{ "data": [ { "id": "gpt-4" }, ... ] }
                    if let Some(arr) = json.get("data").and_then(|v| v.as_array()) {
                        for item in arr {
                            if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                                if !id.is_empty() && !models.contains(&id.to_string()) {
                                    models.push(id.to_string());
                                }
                            }
                        }
                    }
                    // Ollama 格式：{ "models": [ { "name": "llama3" }, ... ] }
                    if let Some(arr) = json.get("models").and_then(|v| v.as_array()) {
                        for item in arr {
                            if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
                                if !name.is_empty() && !models.contains(&name.to_string()) {
                                    models.push(name.to_string());
                                }
                            }
                        }
                    }
                    if !models.is_empty() {
                        return Ok(Json(json!({ "models": models })));
                    }
                    last_err = Some("接口返回了空模型列表，可能该服务不支持 /models 查询".into());
                } else {
                    last_err = Some(format!("HTTP {}", resp.status().as_u16()));
                }
            }
            Err(e) => {
                last_err = Some(format!("请求失败: {e}"));
            }
        }
    }

    Err(AppError::BadRequest(format!(
        "拉取模型列表失败：{}",
        last_err.unwrap_or_else(|| "未知错误".into())
    )))
}

/// 测试 NAS（WebDAV）连接：用前端提交的凭据发 PROPFIND，验证地址/账号/密码是否可连通。
async fn test_nas(
    _user: AuthUser,
    Json(cfg): Json<NasConfig>,
) -> Result<Json<serde_json::Value>, AppError> {
    match crate::agent::tools::nas_tools::test_nas_connection(&cfg).await {
        Ok(count) => Ok(Json(json!({
            "ok": true,
            "item_count": count,
            "message": format!("连接成功，根目录下有 {count} 个文件/目录"),
        }))),
        Err(e) => Ok(Json(json!({
            "ok": false,
            "message": format!("连接失败：{e:#}"),
        }))),
    }
}

/// 测试多模态（图片/视频）模型连接：探测 /models 端点，验证 base_url + api_key 是否连通。
async fn test_media_model(
    _user: AuthUser,
    Json(req): Json<TestMediaReq>,
) -> Result<Json<serde_json::Value>, AppError> {
    let base_url = req.base_url.trim().trim_end_matches('/').to_string();
    if base_url.is_empty() {
        return Err(AppError::BadRequest("Base URL 不能为空".into()));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("构建 HTTP 客户端失败: {e}")))?;

    // 兼容 /models 和 /v1/models
    let endpoints = vec![format!("{base_url}/models"), format!("{base_url}/v1/models")];

    let mut last_err: Option<String> = None;
    for endpoint in endpoints {
        let mut builder = client.get(&endpoint);
        if !req.api_key.trim().is_empty() {
            builder = builder.header("Authorization", format!("Bearer {}", req.api_key.trim()));
        }
        match builder.send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    let json: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
                    let mut names = Vec::new();
                    if let Some(arr) = json.get("data").and_then(|v| v.as_array()) {
                        for item in arr {
                            if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                                names.push(id.to_string());
                            }
                        }
                    }
                    if let Some(arr) = json.get("models").and_then(|v| v.as_array()) {
                        for item in arr {
                            if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
                                names.push(name.to_string());
                            }
                        }
                    }
                    return Ok(Json(json!({
                        "ok": true,
                        "model_count": names.len(),
                        "models": names.iter().take(20).collect::<Vec<_>>(),
                        "message": if names.is_empty() { "连接成功（服务可用，未返回模型列表）".to_string() } else { format!("连接成功，返回 {} 个模型", names.len()) },
                    })));
                } else if status.as_u16() == 401 || status.as_u16() == 403 {
                    last_err = Some(format!("认证失败（HTTP {}），请检查 API Key", status.as_u16()));
                } else {
                    last_err = Some(format!("HTTP {}", status.as_u16()));
                }
            }
            Err(e) => {
                last_err = Some(format!("请求失败: {e}"));
            }
        }
    }

    Ok(Json(json!({
        "ok": false,
        "message": format!("连接失败：{}", last_err.unwrap_or_else(|| "未知错误".into())),
    })))
}

/// 检测模型能力：
/// - text：推理模型连通性 + 是否支持工具调用（function calling）
/// - image：生图模型连通性（调 /v1/images/generations 极小参数）
/// - video：生视频模型连通性（调 /v1/videos 极小参数）
async fn test_llm_capability(
    _user: AuthUser,
    Json(req): Json<TestLlmReq>,
) -> Result<Json<serde_json::Value>, AppError> {
    let base_url = req.base_url.trim().trim_end_matches('/').to_string();
    if base_url.is_empty() {
        return Err(AppError::BadRequest("Base URL 不能为空".into()));
    }

    match req.kind.as_str() {
        "text" => test_text_capability(&base_url, &req.api_key, &req.model).await,
        "image" => test_image_capability(&base_url, &req.api_key, &req.model).await,
        "video" => test_video_capability(&base_url, &req.api_key, &req.model).await,
        _ => Err(AppError::BadRequest("未知检测类型".into())),
    }
}

/// 推理模型检测：连通性 + 工具调用能力
async fn test_text_capability(
    base_url: &str,
    api_key: &str,
    model: &str,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::llm::types::{ChatCompletionRequest, RequestMessage, FunctionDef, FunctionSpec};

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("构建 HTTP 客户端失败: {e}")))?;

    // 1. 连通性 + 工具调用能力：发一个带 tools 的消息，问"现在几点"
    let tool_def = FunctionDef {
        def_type: "function".into(),
        function: FunctionSpec {
            name: "get_current_time".into(),
            description: "获取当前时间".into(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
    };
    let messages = vec![RequestMessage {
        role: "user".into(),
        content: serde_json::Value::String("请调用 get_current_time 工具告诉我现在几点。".into()),
        tool_calls: None,
        tool_call_id: None,
    }];
    let req_body = ChatCompletionRequest {
        model: model.to_string(),
        messages,
        tools: Some(vec![tool_def]),
        tool_choice: None,
        temperature: Some(0.0),
        stream: Some(false),
    };

    let endpoint = build_endpoint(base_url, "chat/completions");
    let resp = client
        .post(&endpoint)
        .header("Authorization", format!("Bearer {}", api_key.trim()))
        .header("Content-Type", "application/json")
        .json(&req_body)
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            return Ok(Json(json!({
                "ok": false,
                "message": format!("连接失败：{e}"),
            })));
        }
    };

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let hint = match status.as_u16() {
            401 | 403 => "认证失败，请检查 API Key".to_string(),
            404 => "端点不存在，可能 base_url 不对".to_string(),
            _ => format!("HTTP {}", status.as_u16()),
        };
        let body_preview: String = body.chars().take(200).collect();
        return Ok(Json(json!({
            "ok": false,
            "message": format!("连接失败：{hint}（{body_preview}）"),
        })));
    }

    let result: crate::llm::types::ChatCompletionResponse = match resp.json().await {
        Ok(r) => r,
        Err(e) => {
            return Ok(Json(json!({
                "ok": false,
                "message": format!("响应解析失败：{e}"),
            })));
        }
    };

    // 判断工具调用能力
    let has_tool_calls = result.choices.iter().any(|c| {
        c.message.tool_calls.as_ref().map(|tc| !tc.is_empty()).unwrap_or(false)
    });

    if has_tool_calls {
        Ok(Json(json!({
            "ok": true,
            "supports_tools": true,
            "message": "检测通过：模型可连通，且具备工具调用（function calling）能力",
        })))
    } else {
        Ok(Json(json!({
            "ok": true,
            "supports_tools": false,
            "message": "模型可连通，但不支持工具调用（未返回 tool_calls）——这会影响 WaLiOffice 的 Agent 工具调度能力",
        })))
    }
}

/// 生图模型检测：调 /v1/images/generations 极小参数
async fn test_image_capability(
    base_url: &str,
    api_key: &str,
    model: &str,
) -> Result<Json<serde_json::Value>, AppError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("构建 HTTP 客户端失败: {e}")))?;

    // 极小参数：按厂商分派（火山方舟 size 用 "2K"，其他用 "1024x1024"）
    let is_volc = crate::agent::tools::agnes_media::detect_video_vendor(base_url)
        == crate::agent::tools::agnes_media::VideoVendor::Volcengine;
    let endpoint = build_endpoint(base_url, "images/generations");
    let body = if is_volc {
        json!({
            "model": model,
            "prompt": "a single red circle",
            "size": "2K",
            "response_format": "url",
            "watermark": true,
        })
    } else {
        json!({
            "model": model,
            "prompt": "a single red circle",
            "size": "1024x1024",
            "n": 1,
        })
    };

    let resp = client
        .post(&endpoint)
        .header("Authorization", format!("Bearer {}", api_key.trim()))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            return Ok(Json(json!({ "ok": false, "message": format!("连接失败：{e}") })));
        }
    };

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let hint = match status.as_u16() {
            401 | 403 => "认证失败，请检查 API Key".to_string(),
            404 => "端点不存在，可能 base_url 或路径不对".to_string(),
            _ => format!("HTTP {}", status.as_u16()),
        };
        let body_preview: String = body.chars().take(200).collect();
        return Ok(Json(json!({
            "ok": false,
            "message": format!("生图失败：{hint}（{body_preview}）"),
        })));
    }

    let result: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
    // 检查是否返回图片（url / b64_json / data[].url / data[].b64_json）
    let has_image = result
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| arr.iter().any(|item| item.get("url").is_some() || item.get("b64_json").is_some()))
        .unwrap_or(false)
        || result.get("url").is_some()
        || result.get("b64_json").is_some()
        || result.get("image_url").is_some()
        || result.get("image_urls").is_some();

    Ok(Json(json!({
        "ok": true,
        "has_image": has_image,
        "message": if has_image { "检测通过：生图模型可用，成功生成图片" } else { "模型响应正常，但未返回图片（响应格式可能不匹配）" },
    })))
}

/// 生视频模型检测：按厂商分派端点（Agnes /v1/videos，火山方舟 /contents/generations/tasks）
async fn test_video_capability(
    base_url: &str,
    api_key: &str,
    model: &str,
) -> Result<Json<serde_json::Value>, AppError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("构建 HTTP 客户端失败: {e}")))?;

    let is_volc = crate::agent::tools::agnes_media::detect_video_vendor(base_url)
        == crate::agent::tools::agnes_media::VideoVendor::Volcengine;

    let endpoint = if is_volc {
        build_endpoint(base_url, "contents/generations/tasks")
    } else {
        build_endpoint(base_url, "videos")
    };
    let body = if is_volc {
        json!({
            "model": model,
            "content": [ { "type": "text", "text": "a single red circle" } ],
            "resolution": "480p",
            "duration": 4,
            "ratio": "16:9",
        })
    } else {
        json!({
            "model": model,
            "prompt": "a single red circle",
        })
    };

    let resp = client
        .post(&endpoint)
        .header("Authorization", format!("Bearer {}", api_key.trim()))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            return Ok(Json(json!({ "ok": false, "message": format!("连接失败：{e}") })));
        }
    };

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let hint = match status.as_u16() {
            401 | 403 => "认证失败，请检查 API Key".to_string(),
            404 => "端点不存在，可能 base_url 或路径不对".to_string(),
            _ => format!("HTTP {}", status.as_u16()),
        };
        let body_preview: String = body.chars().take(200).collect();
        return Ok(Json(json!({
            "ok": false,
            "message": format!("生视频失败：{hint}（{body_preview}）"),
        })));
    }

    let result: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
    // 检查是否返回任务 id
    let has_task = result.get("id").is_some() || result.get("task_id").is_some() || result.get("data").is_some();

    Ok(Json(json!({
        "ok": true,
        "has_task": has_task,
        "message": if has_task { "检测通过：生视频模型可用，成功创建视频任务" } else { "模型响应正常，但未返回任务 id（响应格式可能不匹配）" },
    })))
}

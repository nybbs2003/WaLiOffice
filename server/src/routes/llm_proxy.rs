//! LiteLLM 网关代理：面向登录用户的 API Key 管理（创建/列表/吊销）与模型列表。
//! 后端通过 LITELLM_MASTER_KEY 调用 LiteLLM 管理接口；每个 Key 的 metadata.user_id
//! 记录归属用户，列表按用户过滤、吊销前校验归属。

use axum::extract::Path;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::middleware::AuthUser;
use crate::error::AppError;

pub fn router() -> Router {
    Router::new()
        .route("/api/llm/keys", get(list_keys).post(create_key))
        .route("/api/llm/keys/{key_id}", delete(revoke_key))
        .route("/api/llm/models", get(list_models))
}

fn litellm_target() -> (String, String) {
    let cfg = crate::config::config();
    (
        cfg.litellm_url.trim_end_matches('/').to_string(),
        cfg.litellm_master_key.clone(),
    )
}

async fn llm_request(
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<Value, AppError> {
    let (url, master) = litellm_target();
    if master.is_empty() {
        return Err(AppError::BadRequest(
            "服务未配置 LiteLLM 网关（LITELLM_MASTER_KEY）".into(),
        ));
    }
    let client = reqwest::Client::new();
    let method = match method {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "DELETE" => reqwest::Method::DELETE,
        _ => reqwest::Method::GET,
    };
    let mut req = client
        .request(method, format!("{url}{path}"))
        .header("Authorization", format!("Bearer {master}"))
        .timeout(std::time::Duration::from_secs(25));
    if let Some(body) = body {
        req = req.json(&body);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| AppError::BadRequest(format!("LiteLLM 网关不可达: {e}")))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = value
            .get("error")
            .and_then(|v| v.get("message"))
            .and_then(|v| v.as_str())
            .unwrap_or(text.as_str());
        return Err(AppError::BadRequest(format!(
            "LiteLLM 返回错误 {status}: {msg}"
        )));
    }
    Ok(value)
}

/// 列出当前用户的 API Keys（按 metadata.user_id 过滤）
async fn list_keys(user: AuthUser) -> Result<Json<Value>, AppError> {
    let value = llm_request("GET", "/key/list", None).await?;
    let data = value.get("data").cloned().unwrap_or_default();
    let keys: Vec<Value> = data
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|k| {
            k.get("metadata")
                .and_then(|m| m.get("user_id"))
                .and_then(|v| v.as_str())
                == Some(user.0.id.as_str())
        })
        .collect();
    Ok(Json(json!({ "keys": keys })))
}

#[derive(Debug, Deserialize)]
struct CreateKeyReq {
    name: String,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    budget: Option<f64>,
    #[serde(default)]
    duration: Option<String>,
}

/// 创建 API Key（LiteLLM virtual key，归属当前用户）
async fn create_key(user: AuthUser, Json(req): Json<CreateKeyReq>) -> Result<Json<Value>, AppError> {
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("Key 名称不能为空".into()));
    }
    let mut body = json!({
        "key_alias": format!("{}|{}", user.0.username, name),
        "metadata": {
            "user_id": user.0.id,
            "owner_name": user.0.username,
            "alias": name,
        },
    });
    if !req.models.is_empty() {
        body["models"] = json!(req.models);
    }
    if let Some(budget) = req.budget {
        if budget > 0.0 {
            body["max_budget"] = json!(budget);
        }
    }
    if let Some(duration) = req.duration.as_deref().filter(|d| !d.is_empty()) {
        body["duration"] = json!(duration);
    }
    let value = llm_request("POST", "/key/generate", Some(body)).await?;
    Ok(Json(value))
}

/// 吊销 API Key（先校验归属，防止越权删除他人 Key）
async fn revoke_key(user: AuthUser, Path(key_id): Path<String>) -> Result<Json<Value>, AppError> {
    let info = llm_request("GET", &format!("/key/info?key={key_id}"), None).await?;
    let owner = info
        .get("info")
        .and_then(|i| i.get("metadata"))
        .and_then(|m| m.get("user_id"))
        .and_then(|v| v.as_str());
    if owner != Some(user.0.id.as_str()) {
        return Err(AppError::Forbidden);
    }
    let value = llm_request("POST", "/key/delete", Some(json!({ "keys": [key_id] }))).await?;
    Ok(Json(value))
}

/// 可用的算力模型列表（供 Dashboard 帮助说明展示）
async fn list_models(_user: AuthUser) -> Result<Json<Value>, AppError> {
    let value = llm_request("GET", "/model/info", None).await?;
    Ok(Json(value))
}

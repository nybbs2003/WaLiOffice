use axum::extract::Path;
use axum::extract::Query;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::middleware::AuthUser;
use crate::db::session_repo;
use crate::error::AppError;
use crate::state;

pub fn router() -> Router {
    Router::new()
        .route("/api/chat/sessions", get(list_sessions))
        .route(
            "/api/chat/session/:session_id",
            get(get_session)
                .patch(update_session)
                .delete(delete_session),
        )
        .route("/api/chat/session/:session_id/messages", get(get_messages))
        .route("/api/chat/session/:session_id/clear", post(clear_session))
}

#[derive(Deserialize)]
struct SessionListQuery {
    q: Option<String>,
}

#[derive(Deserialize)]
struct UpdateSessionPayload {
    title: Option<String>,
    project_id: Option<Value>,
    order_col: Option<i64>,
}

async fn list_sessions(
    user: AuthUser,
    Query(query): Query<SessionListQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let pool = state::db_pool();
    tracing::info!("[Sessions] list_sessions called: user_id={}, username={}, query={:?}", user.0.id, user.0.username, query.q);
    let sessions = session_repo::list_by_owner(&pool, &user.0.id, 50, query.q.as_deref()).await?;
    tracing::info!("[Sessions] list_sessions: user={}, count={}", user.0.id, sessions.len());
    if let Some(first) = sessions.first() {
        tracing::info!("[Sessions] first session: id={}, title={}", first.id, first.title);
    }
    let json_val = serde_json::to_value(&sessions).map_err(|e| {
        tracing::error!("[Sessions] serialize error: {:?}", e);
        AppError::Internal(anyhow::anyhow!("serialize error: {}", e))
    })?;
    tracing::info!("[Sessions] serialized OK, len={}", json_val.as_array().map(|a| a.len()).unwrap_or(0));
    Ok(Json(json!({ "sessions": json_val })))
}

async fn get_session(
    user: AuthUser,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let pool = state::db_pool();
    tracing::info!("[Session] get_session called: session_id={}, user={}", session_id, user.0.id);
    let session = session_repo::get_session_detail(&pool, &session_id)
        .await
        .map_err(|e| {
            tracing::error!("[Session] get_session_detail error: {:?}", e);
            e
        })?
        .ok_or(AppError::NotFound("会话不存在".into()))?;
    if session.owner_id != user.0.id {
        return Err(AppError::Forbidden);
    }
    let mut session = session;
    // 历史会话里生成图片存的是厂商签名 URL（会过期），生成时字节已保存到「我的文件」，
    // 这里按 artifact_id 找回本地文件并重写为内部稳定地址，避免对话流预览裂图。
    if let Ok(token) = crate::auth::create_token(&user.0) {
        rewrite_image_artifacts_to_local(&pool, &user.0.id, &token, &mut session.artifacts).await;
    }
    Ok(Json(json!(session)))
}

/// 把图片类产物的外部 URL 重写为本地稳定地址（/api/files/{id}/stream?token=...）；
/// 原始外部 URL 保留到 source_images，供图生图参考。
async fn rewrite_image_artifacts_to_local(
    pool: &crate::db::DbPool,
    user_id: &str,
    token: &str,
    artifacts: &mut [crate::models::Artifact],
) {
    for artifact in artifacts.iter_mut() {
        if artifact.kind != "image" {
            continue;
        }
        let Some(images) = artifact
            .content
            .get("images")
            .and_then(|value| value.as_array())
            .cloned()
        else {
            continue;
        };
        let has_external = images.iter().any(|url| {
            url.as_str()
                .map(|u| !u.starts_with("/api/files/") && !u.starts_with("data:"))
                .unwrap_or(false)
        });
        if !has_external {
            continue;
        }
        let files = match crate::db::file_repo::find_files_by_artifact_id(pool, user_id, &artifact.id).await {
            Ok(files) => files,
            Err(_) => continue,
        };
        if files.is_empty() {
            continue;
        }
        let mut stable: Vec<Value> = Vec::with_capacity(images.len());
        for (idx, url) in images.iter().enumerate() {
            let Some(url_str) = url.as_str() else {
                stable.push(url.clone());
                continue;
            };
            if url_str.starts_with("/api/files/") || url_str.starts_with("data:") {
                stable.push(url.clone());
                continue;
            }
            let file = files
                .iter()
                .find(|f| {
                    f.metadata
                        .as_ref()
                        .and_then(|m| m.get("image_index"))
                        .and_then(|v| v.as_i64())
                        == Some(idx as i64)
                })
                .or_else(|| {
                    // 旧数据没有 image_index：文件数与图片数一致时按位置对齐，否则只救第一张
                    if files.len() == images.len() {
                        files.get(idx)
                    } else if idx == 0 {
                        files.first()
                    } else {
                        None
                    }
                });
            match file {
                Some(file) => {
                    stable.push(json!(format!(
                        "/api/files/{}/stream?token={}",
                        file.id, token
                    )));
                }
                None => stable.push(url.clone()),
            }
        }
        artifact.content["source_images"] = json!(images);
        artifact.content["images"] = json!(stable);
    }
}

async fn get_messages(
    user: AuthUser,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let pool = state::db_pool();
    let session = session_repo::find_by_id(&pool, &session_id)
        .await?
        .ok_or(AppError::NotFound("会话不存在".into()))?;
    if session.owner_id != user.0.id {
        return Err(AppError::Forbidden);
    }
    let messages = session_repo::get_messages(&pool, &session_id, 100).await?;
    Ok(Json(json!({ "messages": messages })))
}

async fn update_session(
    user: AuthUser,
    Path(session_id): Path<String>,
    Json(payload): Json<UpdateSessionPayload>,
) -> Result<Json<serde_json::Value>, AppError> {
    let pool = state::db_pool();
    let mut updated = false;

    if let Some(title) = payload.title {
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err(AppError::BadRequest("标题不能为空".into()));
        }
        updated |= session_repo::update_title(&pool, &session_id, &user.0.id, &title).await?;
    }

    if payload.project_id.is_some() || payload.order_col.is_some() {
        let session = session_repo::find_by_id(&pool, &session_id)
            .await?
            .ok_or(AppError::NotFound("会话不存在".into()))?;
        if session.owner_id != user.0.id {
            return Err(AppError::Forbidden);
        }
        let project_id_owned = match payload.project_id {
            Some(Value::Null) => None,
            Some(Value::String(value)) => {
                Some(value.trim().to_string()).filter(|value| !value.is_empty())
            }
            Some(_) => return Err(AppError::BadRequest("项目 ID 格式不正确".into())),
            None => session.project_id,
        };
        let order_col = payload.order_col.unwrap_or(session.order_col);
        updated |= session_repo::update_project_and_order(
            &pool,
            &session_id,
            &user.0.id,
            project_id_owned.as_deref(),
            order_col,
        ).await?;
    }

    Ok(Json(json!({ "updated": updated })))
}

async fn delete_session(
    user: AuthUser,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let pool = state::db_pool();
    let deleted = session_repo::delete(&pool, &session_id, &user.0.id).await?;
    Ok(Json(json!({ "deleted": deleted })))
}

async fn clear_session(
    user: AuthUser,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let pool = state::db_pool();
    let cleared = session_repo::clear_messages(&pool, &session_id, &user.0.id).await?;
    Ok(Json(json!({ "cleared": cleared })))
}

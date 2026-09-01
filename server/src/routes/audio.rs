use axum::routing::post;
use axum::{Json, Router};
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::auth::middleware::AuthUser;
use crate::error::AppError;

pub fn router() -> Router {
    Router::new()
        .route("/api/audio/recordings", post(create_recording))
        .route("/api/audio/transcribe", post(transcribe_existing))
}

#[derive(Debug, Deserialize)]
struct CreateRecordingReq {
    /// 原始文件名（仅展示用）
    #[serde(default)]
    filename: String,
    /// WAV 音频（base64）
    wav_b64: String,
    /// 录音时长（秒）
    #[serde(default)]
    duration: f32,
    /// 可选：NAS 存放相对路径（缺省 会议录音/YYYYMMDD/xxx.wav）
    #[serde(default)]
    nas_out: String,
}

#[derive(Debug, Serialize)]
struct CreateRecordingResp {
    ok: bool,
    /// NAS 存放路径（WebDAV 写入成功时返回；失败为 null，前端保留 localStorage）
    nas_path: Option<String>,
    /// 转写文本（worker 可用时）
    text: Option<String>,
    /// 转写/存储过程的消息
    message: String,
}

/// 录音上传：写 WebDAV（经多数据源自动路由）+ 本地 worker 转写。
/// 网络/存储失败不丢数据：返回 nas_path=null，前端继续持有 localStorage。
async fn create_recording(
    user: AuthUser,
    Json(req): Json<CreateRecordingReq>,
) -> Result<Json<CreateRecordingResp>, AppError> {
    let wav = base64::engine::general_purpose::STANDARD
        .decode(req.wav_b64.trim())
        .map_err(|_| AppError::BadRequest("wav_b64 不是合法 base64".into()))?;
    if wav.len() < 44 {
        return Err(AppError::BadRequest("音频数据过短".into()));
    }

    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let fname = if req.filename.is_empty() {
        format!("录音_{ts}.wav")
    } else {
        let name = req.filename.trim().replace(['/', '\\'], "_");
        if name.to_lowercase().ends_with(".wav") {
            name
        } else {
            format!("{name}.wav")
        }
    };
    let nas_rel = if req.nas_out.trim().is_empty() {
        format!("会议录音/{}/{}", chrono::Local::now().format("%Y%m%d"), fname)
    } else {
        let p = req.nas_out.trim().trim_matches('/').to_string();
        if p.to_lowercase().ends_with(".wav") {
            p
        } else {
            format!("{p}/{fname}")
        }
    };

    // 1) 写 NAS（多数据源：直连或 worker 中继自动路由；未配置数据源则跳过）
    let mut nas_path: Option<String> = None;
    let mut nas_msg = String::new();
    match crate::agent::tools::nas_tools::write_file_for_user(&user.0.id, &nas_rel, &wav).await {
        Ok(p) => {
            nas_path = Some(p);
        }
        Err(e) => {
            nas_msg = format!("NAS 写入未完成：{e:#}");
        }
    }

    // 2) 本地转写（worker 不可用时降级为空）
    let mut text: Option<String> = None;
    let mut tr_msg = String::new();
    match crate::agent::tools::nas_tools::relay_transcribe(&wav).await {
        Ok(t) => {
            text = Some(t);
        }
        Err(e) => {
            tr_msg = format!("转写未完成：{e:#}");
        }
    }

    let mut parts = Vec::new();
    if nas_path.is_some() {
        parts.push("已存入 WebDAV".to_string());
    } else if !nas_msg.is_empty() {
        parts.push(nas_msg);
    }
    if text.is_some() {
        parts.push("转写完成".to_string());
    } else if !tr_msg.is_empty() {
        parts.push(tr_msg);
    }

    Ok(Json(CreateRecordingResp {
        ok: nas_path.is_some() || text.is_some(),
        nas_path,
        text,
        message: parts.join("；"),
    }))
}

#[derive(Debug, Deserialize)]
struct TranscribeReq {
    /// NAS 上的音频相对路径
    nas_path: String,
}

/// 对已存入 NAS 的音频做转写（录音未上传成功时的补转路径）。
async fn transcribe_existing(
    user: AuthUser,
    Json(req): Json<TranscribeReq>,
) -> Result<Json<serde_json::Value>, AppError> {
    let wav = crate::agent::tools::nas_tools::read_file_for_user(&user.0.id, &req.nas_path)
        .await
        .map_err(|e| AppError::BadRequest(format!("读取 NAS 音频失败：{e}")))?;
    let text = crate::agent::tools::nas_tools::relay_transcribe(&wav)
        .await
        .map_err(|e| AppError::BadRequest(format!("转写失败：{e}")))?;
    Ok(Json(serde_json::json!({ "ok": true, "text": text })))
}


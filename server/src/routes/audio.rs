use axum::routing::post;
use axum::{Json, Router};
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::auth::middleware::AuthUser;
use crate::llm::LlmClient;
use crate::models::ChatMessage;
use crate::error::AppError;

pub fn router() -> Router {
    Router::new()
        .route("/api/audio/recordings", post(create_recording))
        .route("/api/audio/transcribe", post(transcribe_existing))
        .route("/api/audio/stream/{sid}/chunk", post(stream_chunk))
        .route("/api/audio/stream/{sid}/transcript", axum::routing::get(stream_transcript))
        .route("/api/audio/stream/{sid}/finish", post(stream_finish))
        .route("/api/audio/stream/{sid}/minutes", post(stream_minutes))
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



// ---------------- 流式录音会话（分块上传 → worker 滑窗转写 → 增量纪要） ----------------

/// 分块上传：透传 worker（音频字节经 frp 控制通道，不落公网存储）
async fn stream_chunk(
    _user: AuthUser,
    axum::extract::Path(sid): axum::extract::Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    let (status, text) = crate::agent::tools::nas_tools::relay_stream(
        "POST",
        &format!("/stream/{sid}/chunk"),
        Some(body),
    )
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("worker 不可用：{e}")))?;
    if status != 200 {
        return Err(AppError::Internal(anyhow::anyhow!("worker 分块上传失败（{status}）：{}", text.chars().take(200).collect::<String>())));
    }
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::json!({ "ok": true }));
    Ok(Json(v))
}

/// 查询实时转写（滑窗重译，含对前文的更正）
async fn stream_transcript(
    _user: AuthUser,
    axum::extract::Path(sid): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let (status, text) = crate::agent::tools::nas_tools::relay_stream("GET", &format!("/stream/{sid}/transcript"), None)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("worker 不可用：{e}")))?;
    if status != 200 {
        return Err(AppError::Internal(anyhow::anyhow!("worker 查询失败（{status}）")));
    }
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::json!({ "text": "" }));
    Ok(Json(v))
}

/// 录音结束：等转写收敛后返回最终文本
async fn stream_finish(
    _user: AuthUser,
    axum::extract::Path(sid): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let (status, text) = crate::agent::tools::nas_tools::relay_stream("POST", &format!("/stream/{sid}/finish"), Some(serde_json::json!({})))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("worker 不可用：{e}")))?;
    if status != 200 {
        return Err(AppError::Internal(anyhow::anyhow!("worker 结束失败（{status}）")));
    }
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::json!({ "text": "" }));
    Ok(Json(v))
}

#[derive(Debug, serde::Deserialize)]
struct MinutesReq {
    transcript: String,
    /// 上一次的纪要（增量更新：根据后文修正上文）
    #[serde(default)]
    prev: String,
}

/// 实时纪要：根据当前转写（可能已更正）生成/更新会议纪要。
/// 增量模式下要求 LLM 基于后文更新前半部分内容，输出完整新版纪要。
async fn stream_minutes(
    user: AuthUser,
    axum::extract::Path(_sid): axum::extract::Path<String>,
    Json(req): Json<MinutesReq>,
) -> Result<Json<serde_json::Value>, AppError> {
    let transcript = req.transcript.trim().to_string();
    if transcript.is_empty() {
        return Ok(Json(serde_json::json!({ "markdown": "" })));
    }
    let prev = req.prev.trim().to_string();
    let system = if prev.is_empty() {
        "你是会议纪要助手。根据实时会议转写生成结构化 Markdown 纪要：会议主题、议题与要点、结论决策、行动项（事项/负责人/截止，未提及填待定）。转写是流式的可能不完整，只基于已有内容，不要编造。"
    } else {
        "你是会议纪要助手。已有会议纪要，现在会议有了新的转写内容（可能包含对前文转写的更正）。请输出更新后的完整会议纪要：保留既有结构，根据新增内容补充或修改议题、结论、行动项，用后文信息修正与前文矛盾的内容。输出完整 Markdown（不是 diff）。"
    };
    let user_prompt = if prev.is_empty() {
        format!("当前会议转写：

{transcript}")
    } else {
        format!("已有会议纪要：

{prev}

当前完整转写（含对前文的更正）：

{transcript}")
    };

    let planner = LlmClient::for_user(&user.0.id, None).await;
    let resp = match planner
        .chat(
            &[
                ChatMessage { role: "system".into(), content: system.into(), tool_calls: None, tool_call_id: None, reasoning_content: None },
                ChatMessage { role: "user".into(), content: user_prompt, tool_calls: None, tool_call_id: None, reasoning_content: None },
            ],
            None,
        )
        .await
    {
        Ok(r) => r,
        Err(e) => return Err(AppError::Internal(anyhow::anyhow!("纪要生成失败：{e}"))),
    };
    let markdown = resp.choices.first().and_then(|c| c.message.content.as_deref()).unwrap_or("").trim().to_string();
    Ok(Json(serde_json::json!({ "markdown": markdown })))
}

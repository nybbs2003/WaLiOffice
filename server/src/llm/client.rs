use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tracing::debug;

use super::types::*;
use crate::models::{ChatAttachment, ChatMessage, LlmProfileConfig};

static API_KEY_ROUND_ROBIN: Lazy<Mutex<HashMap<String, usize>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// 文本（推理）模型厂商类型（用于 Files API 分派）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextVendor {
    /// Moonshot Kimi（moonshot.cn）：/v1/files，purpose=file-extract/image/video
    Kimi,
    /// DeepSeek（api.deepseek.com）：/files，purpose=user_data（仅图片）
    Deepseek,
    /// 其他 OpenAI 兼容（默认）：尝试 /files
    OpenAi,
}

/// 根据 base_url 域名识别文本模型厂商
pub fn detect_text_vendor(base_url: &str) -> TextVendor {
    let base = base_url.trim().to_lowercase();
    if base.contains("moonshot") || base.contains("kimi") {
        TextVendor::Kimi
    } else if base.contains("deepseek") {
        TextVendor::Deepseek
    } else {
        TextVendor::OpenAi
    }
}

/// 超过此大小的图片走 Files API 上传（避免 base64 内联超请求体限制）
const LARGE_ATTACHMENT_BYTES: usize = 2 * 1024 * 1024; // 2 MiB

/// 解码 base64 数据 URL 的字节（去掉 data:xxx;base64, 前缀）
fn base64_decode(b64: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .ok()
}

fn single_key_vec(api_key: &str) -> Vec<String> {
    let key = api_key.trim();
    if key.is_empty() {
        vec![]
    } else {
        vec![key.to_string()]
    }
}

fn profile_api_keys(profile: &LlmProfileConfig) -> Vec<String> {
    let mut keys = Vec::new();
    for key in &profile.api_keys {
        let key = key.trim();
        if !key.is_empty() && !keys.iter().any(|item: &String| item == key) {
            keys.push(key.to_string());
        }
    }
    if let Some(api_key) = profile.api_key.as_deref() {
        let api_key = api_key.trim();
        if !api_key.is_empty() && !keys.iter().any(|item| item == api_key) {
            keys.push(api_key.to_string());
        }
    }
    keys
}

fn rotate_keys(scope: &str, keys: Vec<String>) -> Vec<String> {
    if keys.len() <= 1 {
        return keys;
    }

    let start = API_KEY_ROUND_ROBIN
        .lock()
        .map(|mut cursors| {
            let cursor = cursors.entry(scope.to_string()).or_insert(0);
            let start = *cursor % keys.len();
            *cursor = (*cursor + 1) % keys.len();
            start
        })
        .unwrap_or(0);

    keys.iter()
        .cycle()
        .skip(start)
        .take(keys.len())
        .cloned()
        .collect()
}

fn apply_profile(
    client: &mut LlmClient,
    profile: &LlmProfileConfig,
    preferred_model: Option<&str>,
    active_model: Option<&str>,
    scope: &str,
) {
    if !profile.base_url.trim().is_empty() {
        client.base_url = profile.base_url.trim_end_matches('/').to_string();
    }

    let keys = profile_api_keys(profile);
    client.api_keys = rotate_keys(scope, keys);

    let requested_model = preferred_model.filter(|model| {
        profile.models.iter().any(|item| item == *model) && is_chat_compatible_model(model)
    });
    client.model = requested_model
        .map(|item| item.to_string())
        .or_else(|| {
            active_model.and_then(|model| {
                if profile.models.iter().any(|item| item == model)
                    && is_chat_compatible_model(model)
                {
                    Some(model.to_string())
                } else {
                    None
                }
            })
        })
        .or_else(|| {
            if is_chat_compatible_model(&profile.default_model) {
                Some(profile.default_model.clone())
            } else {
                profile
                    .models
                    .iter()
                    .find(|item| is_chat_compatible_model(item))
                    .cloned()
            }
        })
        .unwrap_or_else(|| profile.default_model.clone());
}

fn should_retry_with_next_key(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::UNAUTHORIZED
        || status == reqwest::StatusCode::FORBIDDEN
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

pub struct LlmClient {
    http: Client,
    base_url: String,
    api_keys: Vec<String>,
    model: String,
    timeout: Duration,
}

impl LlmClient {
    pub fn new() -> Self {
        let cfg = crate::config::config();
        let http = Client::builder()
            .timeout(Duration::from_millis(cfg.llm_chat_timeout_ms))
            .build()
            .expect("reqwest client build");
        let api_keys = if cfg.llm_api_keys.is_empty() {
            single_key_vec(&cfg.llm_api_key)
        } else {
            cfg.llm_api_keys.clone()
        };
        Self {
            http,
            base_url: cfg.llm_base_url.trim_end_matches('/').to_string(),
            api_keys,
            model: cfg.llm_model.clone(),
            timeout: Duration::from_millis(cfg.llm_chat_timeout_ms),
        }
    }

    pub fn from_profile(
        profile: &LlmProfileConfig,
        preferred_model: Option<&str>,
        scope: &str,
    ) -> Self {
        let mut client = Self::new();
        apply_profile(&mut client, profile, preferred_model, None, scope);
        client
    }

    pub async fn for_user(user_id: &str, preferred_model: Option<&str>) -> Self {
        let mut client = Self::new();
        let pool = crate::state::db_pool();

        let settings = crate::db::settings_repo::find_by_user(&pool, user_id)
            .await
            .ok()
            .flatten();

        if let Some(settings) = settings {
            if let Some(profile) = settings
                .llm_profiles
                .iter()
                .find(|item| item.id == settings.active_profile_id)
            {
                apply_profile(
                    &mut client,
                    profile,
                    preferred_model,
                    Some(settings.active_model.as_str()),
                    &format!("user:{user_id}:profile:{}", profile.id),
                );
            }
        } else if let Some(model) =
            preferred_model.filter(|item| !item.trim().is_empty() && is_chat_compatible_model(item))
        {
            client.model = model.to_string();
        }

        client
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// 文本模型厂商
    pub fn text_vendor(&self) -> TextVendor {
        detect_text_vendor(&self.base_url)
    }

    /// 通过 Files API 上传文件（按厂商分派），返回 file_id。
    /// 用于大图片/视频，避免 base64 内联超请求体限制。
    pub async fn upload_file(
        &self,
        filename: &str,
        bytes: &[u8],
        kind: &str, // image | video | text
    ) -> Result<String> {
        let key = self.api_keys.first().cloned().unwrap_or_default();
        let vendor = self.text_vendor();

        let (url, purpose) = match vendor {
            TextVendor::Kimi => {
                let purpose = match kind {
                    "video" => "video",
                    "image" => "image",
                    _ => "file-extract",
                };
                (format!("{}/files", self.base_url), purpose)
            }
            TextVendor::Deepseek => {
                (format!("{}/files", self.base_url), "user_data")
            }
            TextVendor::OpenAi => {
                (format!("{}/files", self.base_url), "user_data")
            }
        };

        // 用 multipart/form-data 上传
        let part = reqwest::multipart::Part::bytes(bytes.to_vec())
            .file_name(filename.to_string());
        let form = reqwest::multipart::Form::new()
            .text("purpose", purpose.to_string())
            .part("file", part);

        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {key}"))
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("文件上传失败（HTTP {}）: {}", status.as_u16(), body.chars().take(300).collect::<String>()));
        }

        let json: serde_json::Value = resp.json().await?;
        json.get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("文件上传响应缺少 id 字段"))
    }

    /// 非流式 chat（带可选工具）
    pub async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[FunctionDef]>,
    ) -> Result<ChatCompletionResponse> {
        self.chat_with_attachments(messages, tools, None).await
    }

    pub async fn chat_with_attachments(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[FunctionDef]>,
        user_attachments: Option<&[ChatAttachment]>,
    ) -> Result<ChatCompletionResponse> {
        // 上传大图片/视频到 Files API 拿 file_id（按厂商分派），避免 base64 内联超限
        let mut processed: Option<Vec<ChatAttachment>> = None;
        if let Some(attachments) = user_attachments {
            let vendor_str = match self.text_vendor() {
                TextVendor::Kimi => "kimi",
                TextVendor::Deepseek => "deepseek",
                TextVendor::OpenAi => "openai",
            };
            let mut updated = attachments.to_vec();
            for att in updated.iter_mut() {
                if att.file_id.is_some() {
                    continue;
                }
                let should_upload = att.kind == "video"
                    || (att.kind == "image" && att.size > LARGE_ATTACHMENT_BYTES);
                if !should_upload {
                    continue;
                }
                // 从 data_url 提取字节（去掉 data:xxx;base64, 前缀）
                let bytes = att
                    .data_url
                    .as_deref()
                    .and_then(|du| du.split_once(',').map(|(_, b64)| b64))
                    .and_then(|b64| base64_decode(b64));
                if let Some(bytes) = bytes {
                    match self.upload_file(&att.name, &bytes, &att.kind).await {
                        Ok(file_id) => {
                            att.file_id = Some(file_id);
                        }
                        Err(err) => {
                            tracing::warn!("文件上传 Files API 失败，回退 base64 内联: {err}");
                        }
                    }
                }
            }
            processed = Some(updated);
        }

        let attachments_ref = processed.as_deref().or(user_attachments);
        let has_image_attachments = has_multimodal_attachments(attachments_ref);
        let vendor_str = match self.text_vendor() {
            TextVendor::Kimi => "kimi",
            TextVendor::Deepseek => "deepseek",
            TextVendor::OpenAi => "openai",
        };
        let req = ChatCompletionRequest {
            model: self.model.clone(),
            messages: build_request_messages(messages, attachments_ref, vendor_str),
            tools: tools.map(|t| t.to_vec()),
            tool_choice: None,
            temperature: Some(0.7),
            stream: Some(false),
        };

        match self.send_chat_request(&req).await {
            Ok(result) => Ok(result),
            Err(err) if has_image_attachments && tools.is_some() => {
                tracing::warn!(
                    "vision request with tools failed, retrying vision chat without tools: {err}"
                );
                let vision_only_req = ChatCompletionRequest {
                    model: self.model.clone(),
                    messages: build_request_messages(messages, attachments_ref, vendor_str),
                    tools: None,
                    tool_choice: None,
                    temperature: Some(0.7),
                    stream: Some(false),
                };

                match self.send_chat_request(&vision_only_req).await {
                    Ok(result) => Ok(result),
                    Err(vision_err) => {
                        tracing::warn!(
                            "vision-only retry failed, retrying text-only chat: {vision_err}"
                        );
                        self.retry_text_only_after_vision_failure(messages, tools)
                            .await
                    }
                }
            }
            Err(err) if has_image_attachments => {
                tracing::warn!("vision request failed, retrying text-only chat: {err}");
                self.retry_text_only_after_vision_failure(messages, tools)
                    .await
            }
            Err(err) => Err(err),
        }
    }

    async fn retry_text_only_after_vision_failure(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[FunctionDef]>,
    ) -> Result<ChatCompletionResponse> {
        let mut fallback_messages = messages.to_vec();
        fallback_messages.push(ChatMessage {
            role: "system".to_string(),
            content: "当前模型或接口暂未成功处理本次图片视觉输入。不要假装已经看到了图片内容；请明确告知用户当前限制，并优先依据 OCR 文本、图片中的关键区域说明或用户补充描述继续回答。".to_string(),
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
        });
        let fallback_req = ChatCompletionRequest {
            model: self.model.clone(),
            messages: build_request_messages(&fallback_messages, None, "openai"),
            tools: tools.map(|t| t.to_vec()),
            tool_choice: None,
            temperature: Some(0.7),
            stream: Some(false),
        };
        self.send_chat_request(&fallback_req).await
    }

    async fn send_chat_request(
        &self,
        req: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse> {
        let url = format!("{}/chat/completions", self.base_url);
        debug!("LLM chat request to {url}");

        let keys = if self.api_keys.is_empty() {
            vec![String::new()]
        } else {
            self.api_keys.clone()
        };
        let mut last_error: Option<anyhow::Error> = None;

        for (index, api_key) in keys.iter().enumerate() {
            let resp = self
                .http
                .post(&url)
                .header("Authorization", format!("Bearer {api_key}"))
                .header("Content-Type", "application/json")
                .timeout(self.timeout)
                .json(req)
                .send()
                .await;

            let resp = match resp {
                Ok(resp) => resp,
                Err(err) => {
                    last_error = Some(err.into());
                    if index + 1 < keys.len() {
                        continue;
                    }
                    break;
                }
            };

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                let err = anyhow!("LLM 返回错误 {status}: {body}");
                if should_retry_with_next_key(status) && index + 1 < keys.len() {
                    tracing::warn!("LLM key failed with {status}, retrying next key");
                    last_error = Some(err);
                    continue;
                }
                return Err(err);
            }

            let result: ChatCompletionResponse = resp.json().await?;
            return Ok(result);
        }

        Err(last_error.unwrap_or_else(|| anyhow!("当前模型服务未配置可用 API Key")))
    }

    /// 提取 JSON（容错：去 markdown fence、截取首尾花括号）
    pub fn extract_json(text: &str) -> Result<serde_json::Value> {
        let cleaned = text
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        if let Ok(v) = serde_json::from_str::<serde_json::Value>(cleaned) {
            return Ok(v);
        }

        // 尝试截取第一个 { 到最后一个 }
        if let (Some(start), Some(end)) = (cleaned.find('{'), cleaned.rfind('}')) {
            if end > start {
                if let Ok(v) = serde_json::from_str(&cleaned[start..=end]) {
                    return Ok(v);
                }
            }
        }
        // 尝试数组
        if let (Some(start), Some(end)) = (cleaned.find('['), cleaned.rfind(']')) {
            if end > start {
                if let Ok(v) = serde_json::from_str(&cleaned[start..=end]) {
                    return Ok(v);
                }
            }
        }
        Err(anyhow!("模型未返回可解析 JSON"))
    }
}

fn build_request_messages(
    messages: &[ChatMessage],
    user_attachments: Option<&[ChatAttachment]>,
    text_vendor: &str,
) -> Vec<RequestMessage> {
    let latest_user_index = user_attachments
        .filter(|items| has_multimodal_attachments(Some(items)))
        .and_then(|_| messages.iter().rposition(|msg| msg.role == "user"));

    messages
        .iter()
        .enumerate()
        .map(
            |(index, msg)| match (Some(index) == latest_user_index, user_attachments) {
                (true, Some(attachments)) => {
                    RequestMessage::from_multimodal_user_message(msg, attachments, text_vendor)
                }
                _ => RequestMessage::from_chat_message(msg),
            },
        )
        .collect()
}

/// 是否含多模态附件（图片/视频，data_url 内联或 file_id 引用）
fn has_multimodal_attachments(user_attachments: Option<&[ChatAttachment]>) -> bool {
    user_attachments
        .map(|items| {
            items.iter().any(|item| {
                (item.kind == "image" || item.kind == "video")
                    && (item
                        .data_url
                        .as_deref()
                        .map(|value| !value.trim().is_empty())
                        .unwrap_or(false)
                        || item
                            .file_id
                            .as_deref()
                            .map(|value| !value.trim().is_empty())
                            .unwrap_or(false))
            })
        })
        .unwrap_or(false)
}

fn has_image_attachments(user_attachments: Option<&[ChatAttachment]>) -> bool {
    has_multimodal_attachments(user_attachments)
}

fn is_chat_compatible_model(model: &str) -> bool {
    let normalized = model.trim().to_lowercase();
    !(normalized.starts_with("agnes-image-") || normalized.starts_with("agnes-video-"))
}

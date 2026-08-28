use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestMessage {
    pub role: String,
    pub content: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// 思考模型的推理过程（多轮回传，Kimi 保留式思考要求原样保留）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

impl RequestMessage {
    pub fn from_chat_message(msg: &crate::models::ChatMessage) -> Self {
        Self {
            role: msg.role.clone(),
            content: Value::String(msg.content.clone()),
            tool_calls: msg.tool_calls.clone(),
            tool_call_id: msg.tool_call_id.clone(),
            reasoning_content: msg.reasoning_content.clone(),
        }
    }

    pub fn from_multimodal_user_message(
        msg: &crate::models::ChatMessage,
        attachments: &[crate::models::ChatAttachment],
        text_vendor: &str,
    ) -> Self {
        let mut content = Vec::new();

        if !msg.content.trim().is_empty() {
            content.push(json!({
                "type": "text",
                "text": msg.content,
            }));
        }

        for attachment in attachments.iter() {
            // 视频附件：video_url 块（Kimi 支持 ms://file-id 或 URL；DeepSeek 暂不支持视频）
            if attachment.kind == "video" {
                if let Some(file_id) = attachment.file_id.as_deref().filter(|s| !s.is_empty()) {
                    let url = if text_vendor == "kimi" {
                        format!("ms://{file_id}")
                    } else {
                        file_id.to_string()
                    };
                    content.push(json!({
                        "type": "video_url",
                        "video_url": { "url": url },
                    }));
                } else if let Some(url) = attachment.data_url.as_deref().filter(|s| !s.is_empty()) {
                    content.push(json!({
                        "type": "video_url",
                        "video_url": { "url": url },
                    }));
                }
                continue;
            }

            // 图片附件
            if attachment.kind != "image" {
                continue;
            }

            // 有 file_id：按厂商引用（Kimi 用 ms://，DeepSeek 用 file 块）
            if let Some(file_id) = attachment.file_id.as_deref().filter(|s| !s.is_empty()) {
                if text_vendor == "deepseek" {
                    content.push(json!({
                        "type": "file",
                        "file_id": file_id,
                    }));
                } else if text_vendor == "kimi" {
                    content.push(json!({
                        "type": "image_url",
                        "image_url": { "url": format!("ms://{file_id}") },
                    }));
                } else {
                    // 其他 OpenAI 兼容：file_id 引用
                    content.push(json!({
                        "type": "file",
                        "file_id": file_id,
                    }));
                }
                continue;
            }

            // 无 file_id：base64/URL 内联
            if let Some(image_url) = normalize_image_url(attachment) {
                content.push(json!({
                    "type": "image_url",
                    "image_url": {
                        "url": image_url,
                        "detail": "high",
                    }
                }));
            }
        }

        if content.is_empty() {
            return Self::from_chat_message(msg);
        }

        Self {
            role: msg.role.clone(),
            content: Value::Array(content),
            tool_calls: msg.tool_calls.clone(),
            tool_call_id: msg.tool_call_id.clone(),
            reasoning_content: msg.reasoning_content.clone(),
        }
    }
}

fn normalize_image_url(attachment: &crate::models::ChatAttachment) -> Option<String> {
    let value = attachment.data_url.as_deref()?.trim();
    if value.is_empty() {
        return None;
    }
    if value.starts_with("data:") || value.starts_with("http://") || value.starts_with("https://") {
        return Some(value.to_string());
    }

    let mime = if attachment.mime_type.trim().is_empty() {
        "image/png"
    } else {
        attachment.mime_type.trim()
    };
    Some(format!("data:{mime};base64,{value}"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<RequestMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<FunctionDef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDef {
    #[serde(rename = "type")]
    pub def_type: String,
    pub function: FunctionSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    pub choices: Vec<Choice>,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    pub message: ResponseMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// 思考模型（DeepSeek-R1 / Kimi K3 等）的推理过程
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: ToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChunk {
    pub choices: Vec<StreamChoice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChoice {
    pub delta: StreamDelta,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StreamDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<StreamToolCall>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamToolCall {
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<StreamToolFunction>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StreamToolFunction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

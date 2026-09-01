use async_trait::async_trait;
use serde_json::json;

use crate::agent::tool::{OfficeTool, ToolArtifact, ToolContext, ToolResult};
use crate::llm::LlmClient;
use crate::models::ChatMessage;

pub struct MeetingMinutesTool;

#[async_trait]
impl OfficeTool for MeetingMinutesTool {
    fn name(&self) -> &str {
        "meeting_minutes"
    }

    fn description(&self) -> &str {
        "会议纪要：基于会议录音转写文本（或直接提供的对话/讨论文本）生成结构化会议纪要，包括议题、讨论要点、结论与行动项（含负责人与截止时间，若文本中提到）"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "transcript": {
                    "type": "string",
                    "description": "会议录音转写文本或会议讨论原始文本"
                },
                "title": {
                    "type": "string",
                    "description": "会议标题（可选，缺省按内容自动提炼）"
                }
            },
            "required": ["transcript"]
        })
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let transcript = input
            .get("transcript")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if transcript.is_empty() {
            return ToolResult::err("transcript 不能为空");
        }
        let title_hint = input
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        let system = r#"你是专业的会议纪要助手。请根据提供的会议转写文本生成一份结构化会议纪要，输出为 Markdown。
要求：
1. 开头给出会议主题（若未提供标题则自行提炼，格式：**会议主题**：xxx）
2. 按讨论顺序梳理「议题与讨论要点」，每个要点用短句概括，保留关键数字、人名、日期与结论
3. 单列「结论与决策」小节
4. 单列「行动项」表格：| 事项 | 负责人 | 截止时间 |（文本中未提及的填「待定」）
5. 结尾附「风险与待确认事项」（没有则写「无」）
不要编造文本中不存在的事实；转写可能有错别字，按语义修正。"#;

        let user_prompt = if title_hint.is_empty() {
            format!("会议转写文本如下：\n\n{transcript}")
        } else {
            format!("会议标题：{title_hint}\n\n会议转写文本如下：\n\n{transcript}")
        };

        let planner = LlmClient::for_user(&ctx.user_id, ctx.preferred_model.as_deref()).await;
        let response = match planner
            .chat(
                &[
                    ChatMessage {
                        role: "system".into(),
                        content: system.into(),
                        tool_calls: None,
                        tool_call_id: None,
                        reasoning_content: None,
                    },
                    ChatMessage {
                        role: "user".into(),
                        content: user_prompt,
                        tool_calls: None,
                        tool_call_id: None,
                        reasoning_content: None,
                    },
                ],
                None,
            )
            .await
        {
            Ok(r) => r,
            Err(e) => return ToolResult::err(format!("纪要生成失败：{e}")),
        };

        let markdown = response
            .choices
            .first()
            .and_then(|c| c.message.content.as_deref())
            .unwrap_or("")
            .trim()
            .to_string();
        if markdown.is_empty() {
            return ToolResult::err("纪要内容为空");
        }

        let title = if title_hint.is_empty() {
            "会议纪要".to_string()
        } else {
            format!("会议纪要：{title_hint}")
        };

        ToolResult::ok(
            format!("已生成{title}"),
            vec![ToolArtifact {
                kind: "markdown".into(),
                title,
                content: json!({
                    "type": "markdown",
                    "markdown": markdown,
                    "source": "meeting_minutes",
                }),
            }],
        )
    }
}

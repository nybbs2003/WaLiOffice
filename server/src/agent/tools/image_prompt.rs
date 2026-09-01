use async_trait::async_trait;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;
use tokio::time::sleep;

use crate::agent::tool::{OfficeTool, ToolArtifact, ToolContext, ToolResult};
use crate::llm::LlmClient;
use crate::models::{ChatAttachment, ChatMessage};

use super::agnes_media::{http_client, image_model_with_override, post_json_url, resolve_image_credentials, AgnesCredentials};

pub struct ImagePromptTool;

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ImagePromptPlan {
    title: String,
    description: String,
    prompts: Vec<ImageVariant>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ImageVariant {
    style: String,
    prompt: String,
}

#[derive(Debug, Deserialize)]
struct AgnesImageResponse {
    data: Vec<AgnesImageData>,
}

#[derive(Debug, Deserialize)]
struct AgnesImageData {
    url: Option<String>,
    b64_json: Option<String>,
    revised_prompt: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct ImageOutputSpec {
    /// 分辨率档位（1K/1.5K/2K），火山方舟方式1
    size: &'static str,
    /// 宽高比（用于 Agnes ratio 参数 + 提示词约束）
    ratio: &'static str,
    /// 像素尺寸（火山方舟方式2，宽x高）
    pixel_size: &'static str,
}

/// 把配置面板的宽高比映射到具体像素尺寸 + 档位。
fn image_spec_for_ratio(aspect_ratio: &str) -> ImageOutputSpec {
    match aspect_ratio.trim() {
        // Seedream 5.0 起方式2像素下限 3686400（约 3.69M），全部取达标尺寸
        "1:1" => ImageOutputSpec { size: "2K", ratio: "1:1", pixel_size: "2048x2048" },
        "16:9" => ImageOutputSpec { size: "2K", ratio: "16:9", pixel_size: "2560x1440" },
        "9:16" => ImageOutputSpec { size: "2K", ratio: "9:16", pixel_size: "1440x2560" },
        "4:3" => ImageOutputSpec { size: "2K", ratio: "4:3", pixel_size: "2304x1728" },
        "3:4" => ImageOutputSpec { size: "2K", ratio: "3:4", pixel_size: "1728x2304" },
        "2:3" => ImageOutputSpec { size: "2K", ratio: "2:3", pixel_size: "1600x2400" },
        "3:2" => ImageOutputSpec { size: "2K", ratio: "3:2", pixel_size: "2400x1600" },
        _ => ImageOutputSpec { size: "2K", ratio: "1:1", pixel_size: "2048x2048" },
    }
}

fn collect_image_inputs(ctx: &ToolContext, input: &serde_json::Value) -> Vec<String> {
    let mut images = input
        .get("image_urls")
        .or_else(|| input.get("images"))
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if let Some(image) = input
        .get("image_url")
        .or_else(|| input.get("image"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        images.push(image.to_string());
    }

    if images.is_empty() {
        images.extend(ctx.attachments.iter().filter_map(attachment_to_data_url));
    }

    images
}

fn attachment_to_data_url(attachment: &ChatAttachment) -> Option<String> {
    if attachment.kind != "image" {
        return None;
    }
    attachment
        .data_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

/// 把内部文件地址（/api/files/{id}/...）解析成 base64 data URL，供图生图直接传给厂商；
/// 外部 http(s) 地址和 data: 地址原样返回。
async fn normalize_image_input(pool: &crate::db::DbPool, user_id: &str, url: &str) -> String {
    if url.starts_with("/api/files/") {
        let id = url
            .trim_start_matches("/api/files/")
            .split(['/', '?'])
            .next()
            .unwrap_or_default();
        if !id.is_empty() {
            if let Ok(Some(file)) = crate::db::file_repo::get_file(pool, user_id, id).await {
                let path = std::path::PathBuf::from(&file.file_path);
                if let Ok(data) = tokio::fs::read(&path).await {
                    let mime = mime_guess::from_path(&file.name).first_or_octet_stream().to_string();
                    return format!(
                        "data:{};base64,{}",
                        mime,
                        base64::engine::general_purpose::STANDARD.encode(&data)
                    );
                }
            }
        }
    }
    url.to_string()
}

fn infer_image_output_spec(topic: &str) -> ImageOutputSpec {
    let lower = topic.to_lowercase();
    if ["海报", "封面", "竖版", "手机", "poster", "小红书", "短视频"]
        .iter()
        .any(|keyword| lower.contains(keyword))
    {
        image_spec_for_ratio("2:3")
    } else if ["头像", "logo", "方图", "icon", "图标", "社媒配图"]
        .iter()
        .any(|keyword| lower.contains(keyword))
    {
        image_spec_for_ratio("1:1")
    } else {
        image_spec_for_ratio("3:2")
    }
}

/// 判断当前模型是否支持「组图生成」（sequential_image_generation 一次出多张）。
/// 火山方舟官方模型列表：
/// - pro 系列（doubao-seedream-5-0-pro-260628）：仅单图生成，传 sequential 参数会 400
/// - 非 pro（5-0 / 5-0-lite / 4-5 / 4-0）：支持组图生成（文生组图/单张图生组图/多参考图生组图）
/// Agnes（非火山）不支持组图。
fn supports_sequential_generation(is_volc: bool, model: &str) -> bool {
    if !is_volc {
        return false;
    }
    !model.to_lowercase().contains("pro")
}

/// 把多套风格提示词整合成一条组图提示词（供 sequential_image_generation 使用）。
fn build_sequential_prompt(topic: &str, variants: &[ImageVariant]) -> String {
    let parts: Vec<String> = variants
        .iter()
        .enumerate()
        .map(|(i, v)| format!("Image {}: {}", i + 1, v.prompt))
        .collect();
    format!(
        "Generate a set of {} distinct images for this request: {topic}. Generate each image exactly as specified below, keeping a consistent overall subject while varying the described style and composition.\n{}",
        variants.len(),
        parts.join("\n")
    )
}

fn image_src_from_response(image: &AgnesImageData) -> Option<String> {
    image
        .url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            image
                .b64_json
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| format!("data:image/png;base64,{value}"))
        })
}

async fn generate_agnes_image(
    client: &reqwest::Client,
    endpoint: &str,
    credentials: &AgnesCredentials,
    body: &serde_json::Value,
) -> Result<AgnesImageResponse, String> {
    let mut latest_error = String::new();
    for attempt in 1..=2 {
        match post_json_url::<AgnesImageResponse>(client, endpoint, credentials, body).await {
            Ok(response) => return Ok(response),
            Err(err) => {
                latest_error = err.to_string();
                if attempt == 1 {
                    sleep(Duration::from_secs(2)).await;
                }
            }
        }
    }
    Err(latest_error)
}

fn infer_scene_guide(topic: &str) -> &'static str {
    let lower = topic.to_lowercase();
    if ["产品", "发布", "官网", "功能", "品牌", "营销"]
        .iter()
        .any(|keyword| lower.contains(keyword))
    {
        "偏商业视觉表达，强调主体、场景、品牌调性、构图层次与高信息密度。"
    } else if ["技术", "架构", "ai", "agent", "智能体", "数据", "系统"]
        .iter()
        .any(|keyword| lower.contains(keyword))
    {
        "偏科技视觉表达，强调系统感、未来感、数据流、光线层次和精确语义对齐。"
    } else if ["活动", "运营", "增长", "拉新", "转化", "campaign"]
        .iter()
        .any(|keyword| lower.contains(keyword))
    {
        "偏传播海报与活动主视觉，强调视觉冲击、情绪氛围、转化感和层次清晰。"
    } else {
        "默认按可直接商用的高质量视觉方案处理，强调主体、背景、次要细节、光照和构图约束。"
    }
}

fn parse_prompt_plan(content: &str) -> Result<ImagePromptPlan, String> {
    let value = LlmClient::extract_json(content).map_err(|err| format!("提示词解析失败: {err}"))?;
    serde_json::from_value::<ImagePromptPlan>(value)
        .map_err(|err| format!("提示词结构不正确: {err}"))
}

#[async_trait]
impl OfficeTool for ImagePromptTool {
    fn name(&self) -> &str {
        "image_prompt"
    }

    fn description(&self) -> &str {
        "生成图片结果：基于 Agnes Image 2.1 Flash 生成高质量图片，并返回可直接预览的图片链接。"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "topic": { "type": "string", "description": "图片需求描述，例如海报、主视觉、封面、插画、品牌配图；如基于图片修改，请说明修改目标和保留元素" },
                "styles": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "期望风格（可选），如写实、插画、3D、极简商务、赛博科技"
                },
                "mode": {
                    "type": "string",
                    "description": "生成模式：text_to_image 或 image_to_image。用户上传图片并要求改图/换风格/基于参考图生成时使用 image_to_image"
                },
                "image_url": { "type": "string", "description": "图生图参考图片 URL 或 data URL，可选；不传时自动使用本轮上传图片" },
                "image_urls": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "多张图生图参考图片 URL 或 data URL，可选"
                }
            },
            "required": ["topic"]
        })
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let topic = input
            .get("topic")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let styles = input
            .get("styles")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(|item| item.trim().to_string())
                    .filter(|item| !item.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if topic.is_empty() {
            return ToolResult::err("topic 不能为空");
        }

        let scene_guide = infer_scene_guide(topic);
        // 读取配置面板：宽高比 + 风格（aspect_ratio / style）
        let config_aspect_ratio = ctx.get_config::<String>("aspect_ratio");
        let config_style = ctx.get_config::<String>("style");
        let output_spec = match config_aspect_ratio.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(ratio) => image_spec_for_ratio(ratio),
            None => infer_image_output_spec(topic),
        };
        // 配置面板指定风格时，覆盖 styles（用于提示词规划）
        let styles = if styles.is_empty() {
            config_style
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| vec![s.to_string()])
                .unwrap_or_default()
        } else {
            styles
        };
        let image_inputs_raw = collect_image_inputs(ctx, &input);
        // 内部文件地址归一化为 base64，图生图引用不受外部签名过期影响
        let image_inputs = {
            let pool = crate::state::db_pool();
            let mut normalized = Vec::with_capacity(image_inputs_raw.len());
            for url in image_inputs_raw {
                normalized.push(normalize_image_input(&pool, &ctx.user_id, &url).await);
            }
            normalized
        };
        let wants_image_to_image = input
            .get("mode")
            .and_then(|v| v.as_str())
            .map(|mode| {
                mode.eq_ignore_ascii_case("image_to_image") || mode.eq_ignore_ascii_case("img2img")
            })
            .unwrap_or(false)
            || !image_inputs.is_empty();

        ctx.send(
            "state_update",
            json!({
                "phase": "running",
                "step": "规划出图方案",
                "detail": format!("正在为《{topic}》生成高质量出图提示词..."),
                "at": chrono::Utc::now().to_rfc3339(),
            }),
        );

        let system_prompt = r#"你是资深视觉创意总监兼 Agnes 出图提示词专家。只输出严格 JSON，不要 markdown。
返回格式：
{
  "title": "图片标题",
  "description": "整体设计说明，说明这些风格分别适合什么业务场景",
  "prompts": [
    {
      "style": "风格名",
      "prompt": "一条可直接用于 Agnes Image 2.1 Flash 的高质量英文提示词"
    }
  ]
}
要求：
- 默认输出 3 种风格
- 提示词必须清晰包含：主要主体、背景环境、重要次要细节、风格与光照、构图约束
- 文生图遵循：[主体] + [场景/环境] + [风格] + [光照] + [构图] + [质量要求]
- 图生图遵循：[修改要求] + [新风格/新场景] + [添加/移除的元素] + [需要保留的元素]
- 图生图必须明确 preserving the original composition / main subject identity / important layout，除非用户要求大幅重绘
- 提示词要偏成品图，而不是抽象概念
- 商业场景要突出可用于官网首屏、活动海报、发布会 KV、培训封面、社媒配图等真实用途
- 文案使用英文为主，必要时可夹带品牌名或专有名词
- 不要输出多余字段"#;

        let reference_guidance = if wants_image_to_image {
            "参考图约束：必须以用户上传的参考图为首要视觉约束，保持主体身份、脸部特征、年龄感、发型、身体比例、原始构图和关键背景稳定；只修改用户要求的内容。不要换脸、不要新增人物、不要生成乱码文字。\n"
        } else {
            ""
        };

        let user_prompt = if styles.is_empty() {
            format!(
                "需求：{topic}\n场景指导：{scene_guide}\n输出尺寸建议：{}，宽高比 {}\n生成模式：{}\n{reference_guidance}请给出 3 套差异明确且可直接出图的 Agnes 提示词。",
                output_spec.size,
                output_spec.ratio,
                if wants_image_to_image { "图生图，基于用户参考图修改或再创作" } else { "文生图" }
            )
        } else {
            format!(
                "需求：{topic}\n场景指导：{scene_guide}\n输出尺寸建议：{}，宽高比 {}\n生成模式：{}\n{reference_guidance}指定风格：{}\n请优先按这些风格输出 3 套可直接出图的 Agnes 提示词。",
                output_spec.size,
                output_spec.ratio,
                if wants_image_to_image { "图生图，基于用户参考图修改或再创作" } else { "文生图" },
                styles.join(" / ")
            )
        };

        let planner = LlmClient::for_user(&ctx.user_id, ctx.preferred_model.as_deref()).await;
        let plan_response = match planner
            .chat(
                &[
                    ChatMessage {
                        role: "system".into(),
                        content: system_prompt.into(),
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
            Ok(response) => response,
            Err(err) => return ToolResult::err(format!("图片方案规划失败: {err}")),
        };

        let plan_content = plan_response
            .choices
            .first()
            .and_then(|choice| choice.message.content.as_deref())
            .unwrap_or("");
        let plan = match parse_prompt_plan(plan_content) {
            Ok(plan) => plan,
            Err(err) => return ToolResult::err(err),
        };

        let credentials = match resolve_image_credentials(&ctx.user_id).await {
            Ok(credentials) => credentials,
            Err(err) => return ToolResult::err(err.to_string()),
        };
        let image_model = image_model_with_override(&ctx.user_id, ctx.get_config::<String>("model").as_deref()).await;
        let endpoint = credentials.endpoint("images/generations");
        let is_volc = credentials.video_vendor() == crate::agent::tools::agnes_media::VideoVendor::Volcengine;
        // 本地适配层生图耗时 2-5 分钟，客户端超时放宽到 10 分钟
        let client = match http_client(Duration::from_secs(600)) {
            Ok(client) => client,
            Err(err) => return ToolResult::err(format!("初始化图像客户端失败: {err}")),
        };

        // 按厂商构建图片生成请求体。seq_max：Some(n) 表示启用组图模式（一次出 n 张）
        let build_image_body = |prompt: &str, img2img: bool, spec: ImageOutputSpec, seq_max: Option<usize>| -> serde_json::Value {
            if is_volc {
                // 火山方舟 Seedream：size 用「方式2」像素值精确控制宽高比（方式1档位需 prompt 描述比例，不可混用）
                // Seedream 5.0 pro 总像素 [921600, 4624220]，宽高比 [1/16, 16]
                let mut body = json!({
                    "model": image_model.as_str(),
                    "prompt": prompt,
                    "size": spec.pixel_size,
                    "ratio": spec.ratio,
                    "response_format": "url",
                    "watermark": true,
                });
                if let Some(max_images) = seq_max {
                    // 组图模式：仅非 pro 模型支持（pro 传这两个参数会 400）
                    body["sequential_image_generation"] = json!("auto");
                    body["sequential_image_generation_options"] = json!({ "max_images": max_images });
                }
                if img2img && !image_inputs.is_empty() {
                    // 火山方舟图生图：image 参数（string 或 string[]）
                    let image_val = if image_inputs.len() == 1 {
                        json!(image_inputs[0].clone())
                    } else {
                        json!(image_inputs.clone())
                    };
                    body["image"] = image_val;
                }
                body
            } else if img2img {
                // Agnes 图生图
                json!({
                    "model": image_model.as_str(),
                    "prompt": prompt,
                    "size": spec.size,
                    "ratio": spec.ratio,
                    "extra_body": {
                        "response_format": "url",
                        "image": image_inputs.clone()
                    }
                })
            } else {
                // Agnes 文生图
                json!({
                    "model": image_model.as_str(),
                    "prompt": prompt,
                    "size": spec.size,
                    "ratio": spec.ratio,
                    "extra_body": {
                        "response_format": "url"
                    }
                })
            }
        };

        let variants = plan.prompts.into_iter().take(3).collect::<Vec<_>>();
        if variants.is_empty() {
            return ToolResult::err("没有生成可用的图片提示词");
        }

        let mut generated_images = Vec::new();
        let mut generated_variants = Vec::new();
        let mut failed_variants = Vec::new();

        let sequential_ok = supports_sequential_generation(is_volc, &image_model);

        if sequential_ok && variants.len() > 1 {
            // ── 组图模式：一次请求出多张，减少等待 ──
            let merged_prompt = build_sequential_prompt(topic, &variants);
            ctx.send(
                "state_update",
                json!({
                    "phase": "running",
                    "step": "Agnes 图像生成",
                    "detail": format!("组图模式：一次生成 {} 张（{}）...", variants.len(), variants.iter().map(|v| v.style.as_str()).collect::<Vec<_>>().join("/")),
                    "at": chrono::Utc::now().to_rfc3339(),
                }),
            );
            let request_body = build_image_body(&merged_prompt, wants_image_to_image, output_spec, Some(variants.len()));
            match generate_agnes_image(&client, &endpoint, &credentials, &request_body).await {
                Ok(response) => {
                    for (index, image) in response.data.into_iter().enumerate() {
                        if let Some(src) = image_src_from_response(&image) {
                            let variant = variants.get(index);
                            generated_images.push(src.clone());
                            generated_variants.push(json!({
                                "style": variant.map(|v| v.style.clone()).unwrap_or_else(|| format!("组图 {}", index + 1)),
                                "prompt": variant.map(|v| v.prompt.clone()).unwrap_or_default(),
                                "url": src,
                                "revised_prompt": image.revised_prompt,
                            }));
                        } else {
                            failed_variants.push(format!("组图第 {} 张没有返回 url 或 b64_json", index + 1));
                        }
                    }
                }
                Err(err) => {
                    failed_variants.push(format!("组图生成失败：{err}"));
                }
            }
        } else if variants.len() > 1 {
            // ── 不支持组图（pro/Agnes）：并行发多个单图请求，减少串行等待 ──
            ctx.send(
                "state_update",
                json!({
                    "phase": "running",
                    "step": "Agnes 图像生成",
                    "detail": format!("并行生成 {} 张图片...", variants.len()),
                    "at": chrono::Utc::now().to_rfc3339(),
                }),
            );
            let futures = variants.iter().enumerate().map(|(index, variant)| {
                let body = build_image_body(&variant.prompt, wants_image_to_image, output_spec, None);
                let client = client.clone();
                let endpoint = endpoint.clone();
                let credentials = credentials.clone();
                let style = variant.style.clone();
                async move {
                    let result = generate_agnes_image(&client, &endpoint, &credentials, &body).await;
                    (index, style, result)
                }
            });
            let results = futures::future::join_all(futures).await;
            for (index, style, result) in results {
                match result {
                    Ok(response) => {
                        let image = response.data.into_iter().next();
                        let src = image.as_ref().and_then(image_src_from_response);
                        if let Some(src) = src {
                            generated_images.push(src.clone());
                            generated_variants.push(json!({
                                "style": style,
                                "prompt": variants.get(index).map(|v| v.prompt.clone()).unwrap_or_default(),
                                "url": src,
                                "revised_prompt": image.and_then(|item| item.revised_prompt),
                            }));
                        } else {
                            failed_variants.push(format!("第 {} 张图片响应成功，但没有返回 url 或 b64_json", index + 1));
                        }
                    }
                    Err(err) => {
                        failed_variants.push(format!("第 {} 张图片生成失败（{}）：{}", index + 1, style, err));
                    }
                }
            }
        } else {
            // ── 单张：直接生成 ──
            let variant = &variants[0];
            ctx.send(
                "state_update",
                json!({
                    "phase": "running",
                    "step": "Agnes 图像生成",
                    "detail": format!("正在生成图片（{}）...", variant.style),
                    "at": chrono::Utc::now().to_rfc3339(),
                }),
            );
            let request_body = build_image_body(&variant.prompt, wants_image_to_image, output_spec, None);
            match generate_agnes_image(&client, &endpoint, &credentials, &request_body).await {
                Ok(response) => {
                    let image = response.data.into_iter().next();
                    if let Some(src) = image.as_ref().and_then(image_src_from_response) {
                        generated_images.push(src.clone());
                        generated_variants.push(json!({
                            "style": variant.style,
                            "prompt": variant.prompt,
                            "url": src,
                            "revised_prompt": image.and_then(|item| item.revised_prompt),
                        }));
                    } else {
                        failed_variants.push("图片响应成功，但没有返回 url 或 b64_json".to_string());
                    }
                }
                Err(err) => failed_variants.push(format!("图片生成失败：{err}")),
            }
        }

        if generated_images.is_empty() {
            ctx.send(
                "state_update",
                json!({
                    "phase": "running",
                    "step": "Agnes 图像生成",
                    "detail": "常规提示词未拿到图片，正在使用简化提示词兜底重试...",
                    "at": chrono::Utc::now().to_rfc3339(),
                }),
            );
            let fallback_prompt = format!(
                "Create a clear, high quality image for this user request: {topic}. Use a simple strong composition, recognizable main subject, polished lighting, and appealing visual details."
            );
            let fallback_spec = image_spec_for_ratio("1:1");
            let fallback_body = build_image_body(&fallback_prompt, wants_image_to_image, fallback_spec, None);
            match generate_agnes_image(&client, &endpoint, &credentials, &fallback_body)
                .await
            {
                Ok(response) => {
                    if let Some(image) = response.data.into_iter().next() {
                        if let Some(src) = image_src_from_response(&image) {
                            generated_images.push(src.clone());
                            generated_variants.push(json!({
                                "style": "兜底生成",
                                "prompt": fallback_prompt,
                                "url": src,
                                "revised_prompt": image.revised_prompt,
                            }));
                        }
                    }
                }
                Err(err) => failed_variants.push(format!("兜底图片生成失败：{err}")),
            }
        }

        if generated_images.is_empty() {
            let detail = if failed_variants.is_empty() {
                "Agnes 已返回响应，但没有拿到可预览的图片链接".to_string()
            } else {
                format!("Agnes 图片生成失败：{}", failed_variants.join("；"))
            };
            return ToolResult::err(detail);
        }

        let observation = if failed_variants.is_empty() {
            format!("已为《{topic}》生成 {} 张图片结果", generated_images.len())
        } else {
            format!(
                "已为《{topic}》生成 {} 张图片结果；部分候选失败：{}",
                generated_images.len(),
                failed_variants.join("；")
            )
        };

        ToolResult::ok(
            observation,
            vec![ToolArtifact {
                kind: "image".into(),
                title: plan.title.clone(),
                content: json!({
                    "type": "generated_image",
                    "title": plan.title,
                    "description": plan.description,
                    "prompt": generated_variants
                        .first()
                        .and_then(|item| item.get("prompt"))
                        .and_then(|value| value.as_str())
                        .unwrap_or(""),
                    "image_size": output_spec.size,
                    "image_ratio": output_spec.ratio,
                    "generation_mode": if wants_image_to_image { "image_to_image" } else { "text_to_image" },
                    "reference_image_count": image_inputs.len(),
                    "images": generated_images,
                    "variants": generated_variants,
                    "provider": "agnes",
                    "model": image_model,
                }),
            }],
        )
    }
}

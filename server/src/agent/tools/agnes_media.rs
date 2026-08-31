use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct AgnesCredentials {
    pub base_url: String,
    pub api_keys: Vec<String>,
}

impl AgnesCredentials {
    /// 智能拼接端点：
    /// - base_url 若已以已知 action 结尾（/images/generations、/videos、/chat/completions、/responses）
    ///   则视为「完整端点」，直接返回（仅当请求的 action 与之相同）。
    /// - 否则从 base_url 提取版本前缀（/v1、/v2、/v3、/api/vN），拼上相对 action。
    /// - 都没有则回退 OpenAI 标准 /v1。
    pub fn endpoint(&self, path: &str) -> String {
        build_endpoint(&self.base_url, path)
    }

    /// 视频生成厂商
    pub fn video_vendor(&self) -> VideoVendor {
        detect_video_vendor(&self.base_url)
    }

    /// 视频创建任务端点（按厂商分派）
    pub fn video_create_endpoint(&self) -> String {
        match self.video_vendor() {
            VideoVendor::Volcengine => self.endpoint("contents/generations/tasks"),
            VideoVendor::Zhipu => self.endpoint("videos/generations"),
            VideoVendor::Agnes => self.endpoint("videos"),
        }
    }

    /// 视频查询任务端点（按厂商分派）
    pub fn video_query_endpoint(&self, task_id: &str) -> String {
        match self.video_vendor() {
            VideoVendor::Volcengine => self.endpoint(&format!("contents/generations/tasks/{}", urlencoding::encode(task_id))),
            VideoVendor::Zhipu => self.endpoint(&format!("async-result/{}", urlencoding::encode(task_id))),
            VideoVendor::Agnes => self.endpoint(&format!("videos/{}", urlencoding::encode(task_id))),
        }
    }

    /// Round-robin 选一个 Key；只有一个就直接返回
    pub fn pick_key(&self, scope: &str) -> Option<String> {
        if self.api_keys.is_empty() {
            return None;
        }
        if self.api_keys.len() == 1 {
            return Some(self.api_keys[0].clone());
        }
        let cursor = AGNES_KEY_CURSOR
            .lock()
            .ok()
            .and_then(|mut map| {
                let entry = map.entry(scope.to_string()).or_insert(0usize);
                let start = *entry % self.api_keys.len();
                *entry = (*entry + 1) % self.api_keys.len();
                Some(start)
            })
            .unwrap_or(0);
        Some(self.api_keys[cursor].clone())
    }

    /// 按 round-robin 顺序返回所有 Key（用于失败重试）
    pub fn ordered_keys(&self, scope: &str) -> Vec<String> {
        if self.api_keys.len() <= 1 {
            return self.api_keys.clone();
        }
        let start = AGNES_KEY_CURSOR
            .lock()
            .ok()
            .and_then(|mut map| {
                let entry = map.entry(scope.to_string()).or_insert(0usize);
                let s = *entry % self.api_keys.len();
                *entry = (*entry + 1) % self.api_keys.len();
                Some(s)
            })
            .unwrap_or(0);
        self.api_keys
            .iter()
            .cycle()
            .skip(start)
            .take(self.api_keys.len())
            .cloned()
            .collect()
    }
}

static AGNES_KEY_CURSOR: Lazy<Mutex<std::collections::HashMap<String, usize>>> =
    Lazy::new(|| Mutex::new(std::collections::HashMap::new()));

use once_cell::sync::Lazy;

/// 已知的端点 action 后缀（base_url 若以这些结尾，视为「完整端点」）
const KNOWN_ACTIONS: &[&str] = &[
    "images/generations",
    "videos",
    "chat/completions",
    "responses",
    "models",
];

/// 智能拼接端点：
/// 1. base_url 若以已知 action 结尾，且请求的 action 与之相同 → 直接返回 base_url
/// 2. 否则从 base_url 提取版本前缀（/v1、/v2、/v3、/api/vN）拼 action
/// 3. 都没有 → 回退 OpenAI 标准 /v1
pub fn build_endpoint(base_url: &str, path: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    // 统一 path 为无前导斜杠的 action（如 images/generations）
    let action = path.trim().trim_start_matches('/');

    // 1. base_url 是否已以某个 action 结尾
    for known in KNOWN_ACTIONS {
        if base.ends_with(known) {
            // 若请求的就是这个 action，直接用 base_url
            if action == *known || (known.ends_with(action) && !action.is_empty()) {
                return base.to_string();
            }
            // 否则把已知 action 去掉，回到「版本根」再拼
            let root = base.trim_end_matches(known).trim_end_matches('/');
            return format!("{root}/{action}");
        }
    }

    // 2. base_url 以版本前缀结尾（/v1、/v3、/api/v3 等）
    // 匹配 /vN 或 /api/vN 结尾
    let base_no_slash = base;
    if let Some((root, ver)) = extract_version_suffix(base_no_slash) {
        return format!("{root}/{ver}/{action}");
    }

    // 3. 纯域名（无版本），回退 OpenAI 标准 /v1
    format!("{base}/v1/{action}")
}

/// 若 base_url 以 /vN 或 /api/vN 结尾，返回 (根, 版本)；否则 None
fn extract_version_suffix(base: &str) -> Option<(&str, &str)> {
    // 找最后一个 / 分段，判断是否是 v1/v2/v3 或 api/vN
    let segments: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return None;
    }
    let last = *segments.last().unwrap();
    if last.len() >= 2 && last.starts_with('v') && last[1..].chars().all(|c| c.is_ascii_digit()) {
        let root = base.trim_end_matches(last).trim_end_matches('/');
        return Some((root, last));
    }
    None
}

/// 视频生成厂商类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoVendor {
    /// Agnes（agnes-ai.com）：OpenAI 风格 /v1/videos 异步任务
    Agnes,
    /// 火山方舟（volces.com / ark.cn）：/api/v3/contents/generations/tasks 异步任务
    Volcengine,
    /// 智谱 BigModel（bigmodel.cn / z.ai）：/api/paas/v4/videos/generations 异步任务
    Zhipu,
}

/// 根据 base_url 域名自动识别视频厂商
pub fn detect_video_vendor(base_url: &str) -> VideoVendor {
    let base = base_url.trim().to_lowercase();
    if base.contains("volces.com") || base.contains("volcengine") || base.contains("ark.cn") {
        VideoVendor::Volcengine
    } else if base.contains("bigmodel.cn") || base.contains("z.ai") || base.contains("zhipu") {
        VideoVendor::Zhipu
    } else {
        VideoVendor::Agnes
    }
}

/// 读取 per-user 的图片模型配置（settings.image_profiles + active_image_profile_id），env 作为 fallback。
pub async fn resolve_image_credentials(user_id: &str) -> Result<AgnesCredentials> {
    let config = crate::config::config();
    let pool = crate::state::db_pool();

    // 优先读用户自己的启用配置
    if let Ok(Some(settings)) = crate::db::settings_repo::find_by_user(&pool, user_id).await {
        if let Some(p) = active_media_profile(&settings, &settings.active_image_profile_id, &settings.image_profiles) {
            if !p.base_url.trim().is_empty() {
                return credentials_from_config(&p.base_url, &p.api_keys, &p.api_key, "图片");
            }
        }
        let p = &settings.image_profile;
        if !p.base_url.trim().is_empty() {
            return credentials_from_config(&p.base_url, &p.api_keys, &p.api_key, "图片");
        }
    }

    // fallback：环境变量
    credentials_from_config(
        &config.llm_image_base_url,
        &config.llm_image_api_keys,
        &config.llm_image_api_key,
        "图片",
    )
}

/// 读取 per-user 的视频模型配置（settings.video_profiles + active_video_profile_id），env 作为 fallback。
pub async fn resolve_video_credentials(user_id: &str) -> Result<AgnesCredentials> {
    let config = crate::config::config();
    let pool = crate::state::db_pool();

    if let Ok(Some(settings)) = crate::db::settings_repo::find_by_user(&pool, user_id).await {
        if let Some(p) = active_media_profile(&settings, &settings.active_video_profile_id, &settings.video_profiles) {
            if !p.base_url.trim().is_empty() {
                return credentials_from_config(&p.base_url, &p.api_keys, &p.api_key, "视频");
            }
        }
        let p = &settings.video_profile;
        if !p.base_url.trim().is_empty() {
            return credentials_from_config(&p.base_url, &p.api_keys, &p.api_key, "视频");
        }
    }

    credentials_from_config(
        &config.llm_video_base_url,
        &config.llm_video_api_keys,
        &config.llm_video_api_key,
        "视频",
    )
}

/// 从多配置列表中取启用项（active id 优先，否则第一项）。
fn active_media_profile<'a>(
    settings: &'a crate::models::AppSettings,
    active_id: &str,
    profiles: &'a [crate::models::MediaProfileConfig],
) -> Option<&'a crate::models::MediaProfileConfig> {
    if !active_id.trim().is_empty() {
        if let Some(p) = profiles.iter().find(|p| p.id == active_id) {
            return Some(p);
        }
    }
    profiles.first()
}

fn credentials_from_config(
    base_url: &str,
    api_keys: &[String],
    fallback_api_key: &str,
    kind: &str,
) -> Result<AgnesCredentials> {
    let base_url = base_url.trim();
    if base_url.is_empty() {
        return Err(anyhow!("未配置 {kind} 模型的 BASE_URL（请在「设置 → {kind}模型」中填写，或配置环境变量）"));
    }

    let mut keys: Vec<String> = api_keys
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // 去重
    keys.dedup();

    if keys.is_empty() {
        let fallback = fallback_api_key.trim();
        if fallback.is_empty() {
            return Err(anyhow!("未配置 {kind} 模型的 API_KEY（请在「设置 → {kind}模型」中填写，或配置环境变量）"));
        }
        keys.push(fallback.to_string());
    }

    Ok(AgnesCredentials {
        base_url: base_url.to_string(),
        api_keys: keys,
    })
}

/// 读取 per-user 的图片模型名（启用的图片配置），env fallback。
pub async fn agnes_image_model(user_id: &str) -> String {
    let config = crate::config::config();
    let pool = crate::state::db_pool();
    if let Ok(Some(settings)) = crate::db::settings_repo::find_by_user(&pool, user_id).await {
        if let Some(p) = active_media_profile(&settings, &settings.active_image_profile_id, &settings.image_profiles) {
            let m = p.model.trim().to_string();
            if !m.is_empty() {
                return m;
            }
        }
        let m = settings.image_profile.model.trim().to_string();
        if !m.is_empty() {
            return m;
        }
    }
    config.llm_image_model.trim().to_string()
}

/// 读取 per-user 的视频模型名（启用的视频配置），env fallback。
pub async fn agnes_video_model(user_id: &str) -> String {
    let config = crate::config::config();
    let pool = crate::state::db_pool();
    if let Ok(Some(settings)) = crate::db::settings_repo::find_by_user(&pool, user_id).await {
        if let Some(p) = active_media_profile(&settings, &settings.active_video_profile_id, &settings.video_profiles) {
            let m = p.model.trim().to_string();
            if !m.is_empty() {
                return m;
            }
        }
        let m = settings.video_profile.model.trim().to_string();
        if !m.is_empty() {
            return m;
        }
    }
    config.llm_video_model.trim().to_string()
}

pub fn http_client(timeout: Duration) -> Result<Client> {
    Ok(Client::builder().timeout(timeout).build()?)
}

fn should_retry_agnes(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::UNAUTHORIZED
        || status == reqwest::StatusCode::FORBIDDEN
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

/// 带 API Key 负载均衡的 POST 请求
pub async fn post_json<T: DeserializeOwned>(
    client: &Client,
    url: &str,
    credentials: &AgnesCredentials,
    body: &Value,
) -> Result<T> {
    post_json_url(client, url, credentials, body).await
}

pub async fn post_json_url<T: DeserializeOwned>(
    client: &Client,
    url: &str,
    credentials: &AgnesCredentials,
    body: &Value,
) -> Result<T> {
    let keys = credentials.ordered_keys("agnes-post");
    if keys.is_empty() {
        return Err(anyhow!("Agnes API 未配置可用 API Key"));
    }

    let mut last_error: Option<anyhow::Error> = None;

    for (index, api_key) in keys.iter().enumerate() {
        let response = client
            .post(url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await;

        match response {
            Ok(resp) => {
                if !resp.status().is_success() {
                    let status = resp.status();
                    let detail = resp.text().await.unwrap_or_default();
                    let err = anyhow!("Agnes API 返回错误 {status}: {detail}");
                    if should_retry_agnes(status) && index + 1 < keys.len() {
                        tracing::warn!(
                            "Agnes API key {} failed with {status}, retrying next key",
                            index
                        );
                        last_error = Some(err);
                        continue;
                    }
                    return Err(err);
                }
                return Ok(resp.json::<T>().await?);
            }
            Err(err) => {
                last_error = Some(err.into());
                if index + 1 < keys.len() {
                    continue;
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("Agnes API 请求失败")))
}

/// 带 API Key 负载均衡的 GET 请求
pub async fn get_json<T: DeserializeOwned>(
    client: &Client,
    url: &str,
    credentials: &AgnesCredentials,
) -> Result<T> {
    let keys = credentials.ordered_keys("agnes-get");
    if keys.is_empty() {
        return Err(anyhow!("Agnes API 未配置可用 API Key"));
    }

    let mut last_error: Option<anyhow::Error> = None;

    for (index, api_key) in keys.iter().enumerate() {
        let response = client
            .get(url)
            .header("Authorization", format!("Bearer {api_key}"))
            .send()
            .await;

        match response {
            Ok(resp) => {
                if !resp.status().is_success() {
                    let status = resp.status();
                    let detail = resp.text().await.unwrap_or_default();
                    let err = anyhow!("Agnes API 返回错误 {status}: {detail}");
                    if should_retry_agnes(status) && index + 1 < keys.len() {
                        tracing::warn!(
                            "Agnes API key {} failed with {status}, retrying next key",
                            index
                        );
                        last_error = Some(err);
                        continue;
                    }
                    return Err(err);
                }
                return Ok(resp.json::<T>().await?);
            }
            Err(err) => {
                last_error = Some(err.into());
                if index + 1 < keys.len() {
                    continue;
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("Agnes API 请求失败")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_endpoint_openai_standard() {
        // 纯域名 → /v1/action
        assert_eq!(
            build_endpoint("https://api.openai.com", "chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            build_endpoint("https://api.openai.com/v1", "images/generations"),
            "https://api.openai.com/v1/images/generations"
        );
    }

    #[test]
    fn build_endpoint_volcengine_v3() {
        // 火山方舟 /api/v3（版本前缀 /v3）
        assert_eq!(
            build_endpoint("https://ark.cn-beijing.volces.com/api/v3", "images/generations"),
            "https://ark.cn-beijing.volces.com/api/v3/images/generations"
        );
        assert_eq!(
            build_endpoint("https://ark.cn-beijing.volces.com/api/v3", "chat/completions"),
            "https://ark.cn-beijing.volces.com/api/v3/chat/completions"
        );
    }

    #[test]
    fn build_endpoint_full_action() {
        // base_url 已含完整 action → 直接返回
        assert_eq!(
            build_endpoint("https://ark.cn-beijing.volces.com/api/v3/images/generations", "images/generations"),
            "https://ark.cn-beijing.volces.com/api/v3/images/generations"
        );
        // base_url 以 /responses 结尾，请求 images/generations → 去掉 responses 回到根再拼
        assert_eq!(
            build_endpoint("https://ark.cn-beijing.volces.com/api/v3/responses", "images/generations"),
            "https://ark.cn-beijing.volces.com/api/v3/images/generations"
        );
    }
}

#[cfg(test)]
mod vendor_tests {
    use super::*;

    #[test]
    fn detect_vendor_by_domain() {
        assert_eq!(detect_video_vendor("https://apihub.agnes-ai.com"), VideoVendor::Agnes);
        assert_eq!(detect_video_vendor("https://ark.cn-beijing.volces.com/api/v3"), VideoVendor::Volcengine);
        assert_eq!(detect_video_vendor("https://www.volcengine.com/api/v3"), VideoVendor::Volcengine);
        // 未知域名默认 Agnes
        assert_eq!(detect_video_vendor("https://api.openai.com/v1"), VideoVendor::Agnes);
    }

    #[test]
    fn video_endpoints_by_vendor() {
        let agnes = AgnesCredentials { base_url: "https://apihub.agnes-ai.com".into(), api_keys: vec![] };
        assert_eq!(agnes.video_create_endpoint(), "https://apihub.agnes-ai.com/v1/videos");
        assert_eq!(agnes.video_query_endpoint("abc"), "https://apihub.agnes-ai.com/v1/videos/abc");

        let volc = AgnesCredentials { base_url: "https://ark.cn-beijing.volces.com/api/v3".into(), api_keys: vec![] };
        assert_eq!(volc.video_create_endpoint(), "https://ark.cn-beijing.volces.com/api/v3/contents/generations/tasks");
        assert_eq!(volc.video_query_endpoint("abc"), "https://ark.cn-beijing.volces.com/api/v3/contents/generations/tasks/abc");
    }
}

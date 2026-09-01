use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub tenant_id: Option<String>,
    pub username: String,
    pub email: Option<String>,
    pub avatar: Option<String>,
    /// 飞书昵称（展示用；无昵称时回退 username）
    #[serde(default)]
    pub nickname: Option<String>,
    pub role: String,
}

impl User {
    /// 是否平台级超级管理员
    pub fn is_super_admin(&self) -> bool {
        self.role == "super_admin"
    }

    /// 是否租户管理员（或超管）
    pub fn is_tenant_admin(&self) -> bool {
        matches!(self.role.as_str(), "super_admin" | "tenant_admin")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tenant {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub plan: String,
    pub status: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub invite_code: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateTenantRequest {
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub plan: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateTenantRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TenantMemberRequest {
    pub user_id: String,
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateUserRoleRequest {
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub user: User,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmProfileConfig {
    pub id: String,
    pub name: String,
    pub base_url: String,
    #[serde(default)]
    pub api_keys: Vec<String>,
    pub models: Vec<String>,
    pub default_model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default)]
    pub has_api_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicSettings {
    pub app_name: String,
    pub workspace_title: String,
    pub brand_tagline: String,
    pub default_theme: String,
}

/// 多模态（图片/视频）模型配置（per-user，存 user_settings）
/// 支持多个配置实例（对齐推理模型 llm_profiles），随时切换启用。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MediaProfileConfig {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_keys: Vec<String>,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub default_model: String,
    #[serde(default)]
    pub has_api_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,
    pub transport: String,
    pub endpoint: String,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatAttachment {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub mime_type: String,
    #[serde(default)]
    pub size: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_url: Option<String>,
    /// 通过 Files API 上传后得到的 file_id（大图片/视频，避免 base64 内联超限）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub llm_profiles: Vec<LlmProfileConfig>,
    pub active_profile_id: String,
    pub default_model: String,
    pub active_model: String,
    pub basic: BasicSettings,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    #[serde(default)]
    pub search_providers: SearchProvidersConfig,
    #[serde(default)]
    pub feishu_token: FeishuToken,
    #[serde(default)]
    pub nas_config: NasConfig,
    #[serde(default)]
    pub image_profile: MediaProfileConfig,
    #[serde(default)]
    pub video_profile: MediaProfileConfig,
    #[serde(default)]
    pub image_profiles: Vec<MediaProfileConfig>,
    #[serde(default)]
    pub active_image_profile_id: String,
    #[serde(default)]
    pub video_profiles: Vec<MediaProfileConfig>,
    #[serde(default)]
    pub active_video_profile_id: String,
    pub updated_at: String,
}

/// 每用户的搜索服务配置（多租户场景下各自填写自己的 key）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchProvidersConfig {
    #[serde(default)]
    pub tavily_api_key: String,
    #[serde(default)]
    pub brave_api_key: String,
    #[serde(default)]
    pub kimi_api_key: String,
    #[serde(default)]
    pub provider: String, // 优先使用的搜索源：auto / tavily / brave / kimi / duckduckgo
}

/// 飞书用户授权令牌（持久化，支持按需增量授权 + 自动刷新）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FeishuToken {
    #[serde(default)]
    pub user_access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    /// access_token 过期时间（Unix 秒）
    #[serde(default)]
    pub expires_at: i64,
    /// refresh_token 过期时间（Unix 秒）
    #[serde(default)]
    pub refresh_expires_at: i64,
    /// 已授权的 scope（空格分隔）
    #[serde(default)]
    pub scopes: String,
    #[serde(default)]
    pub open_id: String,
}

/// 每用户的 NAS（懒猫微服 WebDAV）访问凭据
/// 通过 HTTP(S) WebDAV 协议直接访问 NAS 文件，**不在文件系统上挂载**。
/// 多租户场景：每个用户单独保存自己的懒猫账号 WebDAV 凭据（用户名/密码），
/// 懒猫微服本身按账号隔离文件空间，凭据即命名空间，多用户互不冲突。
/// 凭据按 user_id 隔离存储在 user_settings，工具调用时按 ctx.user_id 读取自己的凭据。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NasConfig {
    /// 数据源名称（展示用）
    #[serde(default)]
    pub name: String,
    /// WebDAV 基础地址，如 https://xxx.heiyu.space/dav
    #[serde(default)]
    pub base_url: String,
    /// WebDAV 用户名（懒猫账号各自的 WebDAV 账号）
    #[serde(default)]
    pub username: String,
    /// WebDAV 密码
    #[serde(default)]
    pub password: String,
    /// 是否已配置
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VerificationLoginRequest {
    pub code: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FeishuLoginRequest {
    /// 飞书授权回调返回的 code
    pub code: String,
    #[serde(default)]
    pub tenant_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InviteRequest {
    /// 邀请码 / 邀请 token
    pub invite_code: String,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub tenant_id: Option<String>,
}

// ── Chat ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// 思考模型（DeepSeek-R1 / Kimi K3 等）的推理过程，多轮工具调用需原样回传
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedChatMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<ChatAttachment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_config: Option<serde_json::Value>,
}

// ── Slide / PPT ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlideElement {
    #[serde(rename = "type")]
    pub element_type: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub bold: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub italic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valign: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cols: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_data: Option<Vec<Vec<String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chart_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chart_data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Slide {
    pub id: String,
    pub layout: String,
    pub background: String,
    pub elements: Vec<SlideElement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PptProject {
    pub id: String,
    pub title: String,
    pub theme: String,
    pub slides: Vec<Slide>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<Vec<serde_json::Value>>,
    pub layout: String,
    pub created_at: String,
    pub updated_at: String,
    pub owner_id: String,
}

// ── Artifact ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub kind: String,
    pub tool_kind: String,
    pub title: String,
    pub status: String,
    pub content: serde_json::Value,
    pub version: i32,
    pub created_at: String,
    pub updated_at: String,
}

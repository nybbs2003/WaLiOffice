// ===== 工具智能体 =====
export type ToolKind = 'general' | 'ppt' | 'doc' | 'drawio' | 'excel' | 'image' | 'video' | 'code';

export type ArtifactKind = 'document' | 'markdown' | 'ppt' | 'drawio' | 'sheet' | 'chart' | 'image' | 'video' | 'code' | 'mixed' | 'search';

export interface ChatAttachment {
  id: string;
  name: string;
  kind: 'text' | 'image' | 'video';
  mime_type: string;
  size: number;
  text_content?: string;
  data_url?: string;
  file_id?: string;
  original_size?: number;
  width?: number;
  height?: number;
  compressed?: boolean;
  from_artifact?: boolean;
}

/** @ 引用产物的标签（输入框内显示为 chip） */
export interface InputRef {
  id: string;
  artifactId: string;
  kind: ArtifactKind;
  title: string;
  /** 发送时拼接的引用文本 */
  refText: string;
  /** 来源会话 ID（当前会话或历史会话） */
  sessionId?: string;
  /** 产物内容摘要（发送时传给后端，让 AI 识别引用内容） */
  contentSummary?: string;
}

export interface LLMProfile {
  id: string;
  name: string;
  base_url: string;
  api_keys?: string[];
  models: string[];
  default_model: string;
  api_key?: string;
  has_api_key?: boolean;
}

export interface BasicSettings {
  app_name: string;
  workspace_title: string;
  brand_tagline: string;
  default_theme: string;
}

export interface MCPServiceConfig {
  id: string;
  name: string;
  transport: string;
  endpoint: string;
  enabled: boolean;
  description?: string;
}

// ===== 搜索服务配置（每用户各自的 API Key） =====
export interface SearchProvidersConfig {
  tavily_api_key: string;
  brave_api_key: string;
  kimi_api_key: string;
  provider: string;
}

// ===== NAS（懒猫微服 WebDAV）访问凭据（每用户各自保存） =====
export interface TtsSettings {
  enabled: boolean;
  auto_play: boolean;
  voice: string;
  rate: string;
  pitch: string;
}

export interface NasConfig {
  name: string;
  base_url: string;
  username: string;
  password: string;
  enabled: boolean;
  mode?: string;
  worker_url?: string;
  worker_key?: string;
}

export interface AppSettings {
  llm_profiles: LLMProfile[];
  active_profile_id: string;
  default_model: string;
  active_model: string;
  basic: BasicSettings;
  mcp_servers: MCPServiceConfig[];
  search_providers?: SearchProvidersConfig;
  nas_config?: NasConfig;
  nas_configs?: NasConfig[];
  tts?: TtsSettings;
  artifact_panel_behavior?: string;
  image_profile?: MediaProfileConfig;
  video_profile?: MediaProfileConfig;
  image_profiles?: MediaProfileConfig[];
  active_image_profile_id?: string;
  video_profiles?: MediaProfileConfig[];
  active_video_profile_id?: string;
  updated_at: string;
}

// ===== 多模态（图片/视频）模型配置（per-user，支持多配置随时切换） =====
export interface MediaProfileConfig {
  id: string;
  name: string;
  base_url: string;
  api_keys: string[];
  api_key: string;
  models: string[];
  model: string;
  default_model: string;
  has_api_key?: boolean;
}

export interface Artifact {
  id: string;
  kind: ArtifactKind;
  tool_kind: ToolKind;
  title: string;
  status: 'draft' | 'generating' | 'ready' | 'error';
  content: any;
  version: number;
  created_at: string;
  updated_at: string;
}

export interface ToolConfigOption {
  key: string;
  label: string;
  type: 'select' | 'toggle';
  options?: Array<{ value: string; label: string; description?: string }>;
  defaultValue: string | boolean;
}

/** 模型下拉的动态选项集（来自用户多媒体配置）：选项 + 默认模型（设置里启用配置的默认模型） */
export interface ModelOptionSet {
  options: NonNullable<ToolConfigOption['options']>;
  defaultModel: string;
}

export interface AgentToolConfig {
  id: ToolKind;
  name: string;
  shortName: string;
  description: string;
  artifactLabel: string;
  promptPlaceholder: string;
  examples: string[];
  configOptions?: ToolConfigOption[];
}

export type ToolConfigMap = Partial<Record<string, string | boolean>>;

export interface ConversationRecord {
  id: string;
  title: string;
  tool: ToolKind;
  summary?: string;
  updated_at: string;
  message_count: number;
  order_col?: number;
  project_id?: string;
  project_title?: string;
}

export interface ProjectMeta {
  id: string;
  title: string;
  description?: string;
  tool_kind?: ToolKind;
  session_count: number;
  sessions?: ConversationRecord[];
  created_at: string;
  updated_at: string;
}

export interface PersistedSession {
  id: string;
  owner_id: string;
  messages: Array<{
    role: 'user' | 'assistant' | 'system' | 'tool';
    content: string;
    tool_calls?: Array<{ function?: { name?: string; arguments?: string } }>;
    tool_call_id?: string;
    created_at?: string;
  }>;
  artifacts?: Artifact[];
  project_id?: string;
  tool_kind?: ToolKind;
  title: string;
  summary?: string;
  order_col?: number;
  created_at: string;
  updated_at: string;
}

/** 共享类型定义 - aippt.xiaofuge.cn */

// ===== 幻灯片元素 =====
export interface SlideElement {
  type: 'text' | 'image' | 'shape' | 'table' | 'chart';
  x: number;
  y: number;
  w: number;
  h: number;
  text?: string;
  fontSize?: number;
  color?: string;
  bold?: boolean;
  italic?: boolean;
  align?: 'left' | 'center' | 'right';
  valign?: 'top' | 'middle' | 'bottom';
  fill?: string;
  path?: string;
  shape?: string;
  rows?: number;
  cols?: number;
  table_data?: string[][];
  chart_type?: string;
  chart_data?: any;
}

// ===== 幻灯片 =====
export interface Slide {
  id: string;
  index: number;
  layout: 'title' | 'content' | 'two-column' | 'image' | 'chart' | 'section';
  background: string;
  elements: SlideElement[];
  notes?: string;
  title?: string;
}

// ===== 绘制历史 =====
export interface ProjectHistoryEntry {
  id: string;
  type: 'create' | 'plan' | 'draw' | 'layout' | 'export' | 'system';
  title: string;
  detail?: string;
  slide_index?: number;
  slide_title?: string;
  created_at: string;
}

// ===== PPT 项目 =====
export interface PPTProject {
  id: string;
  title: string;
  theme: string;
  slides: Slide[];
  history?: ProjectHistoryEntry[];
  layout: '16x9' | '4x3';
  created_at: string;
  updated_at: string;
  owner_id?: string;
}

// ===== 对话消息 =====
export interface ChatMessage {
  role: 'user' | 'assistant';
  content: string;
  timestamp: string;
  action?: string;
  slide_id?: string;
  attachments?: ChatAttachment[];
  /** 用户消息引用的产物标签 */
  inputRefs?: InputRef[];
}

// ===== 用户 =====
export interface User {
  id: string;
  tenant_id?: string | null;
  username: string;
  email?: string;
  avatar?: string;
  /** 飞书昵称（展示用） */
  nickname?: string;
  role?: string;
}

// ===== 飞书风格头像（无头像图片时：按昵称哈希取底色 + 首字）=====
const FEISHU_AVATAR_COLORS = ['#5B8FF9', '#61DDAA', '#65789B', '#F6BD16', '#7262FD', '#78D3F8', '#9661BC', '#F6903D', '#008685', '#F08BB4'];
export function feishuAvatarColor(name: string): string {
  let h = 0;
  for (let i = 0; i < name.length; i++) {
    h = (h * 31 + name.charCodeAt(i)) >>> 0;
  }
  return FEISHU_AVATAR_COLORS[h % FEISHU_AVATAR_COLORS.length];
}
export function feishuAvatarInitial(name: string): string {
  const n = (name || '').trim();
  return (n.charAt(0) || 'U').toUpperCase();
}

// ===== 租户 =====
export interface Tenant {
  id: string;
  name: string;
  slug: string;
  plan: string;
  status: string;
  invite_code?: string;
  created_at: string;
  updated_at: string;
}

export interface TokenResponse {
  access_token: string;
  token_type: string;
  user: User;
}

// ===== Agent Trace =====
export type AgentTraceKind = 'state' | 'tool' | 'artifact' | 'error';

export interface AgentTraceEvent {
  id: string;
  kind: AgentTraceKind;
  title: string;
  detail?: string;
  status: 'running' | 'success' | 'error';
  tool?: string;
  at: string;
}

// ===== SSE 事件 =====
export interface ChatSSEEvent {
  event: 'message' | 'slide_update' | 'project_update' | 'artifact_update' | 'tool_result' | 'state_update' | 'done' | 'error';
  data: any;
}

// ===== 文件管理 =====
export interface FileItem {
  id: string;
  name: string;
  file_type: string;
  file_size: number;
  folder_id?: string;
  description?: string;
  metadata?: Record<string, any>;
  created_at: string;
  updated_at: string;
}

export interface Folder {
  id: string;
  owner_id: string;
  name: string;
  parent_id?: string;
  created_at: string;
  updated_at: string;
}

export interface FileStats {
  by_type: Record<string, number>;
  total_size: number;
  total_files: number;
}

// ===== 通知 =====
export interface Notification {
  id: string;
  type: 'system' | 'project' | 'message';
  title: string;
  content?: string;
  is_read: boolean;
  link?: string;
  created_at: string;
}


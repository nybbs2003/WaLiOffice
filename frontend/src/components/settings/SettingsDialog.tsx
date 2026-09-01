import { Check, KeyRound, Plus, Trash2, X, Cpu, Search, Loader2, HardDrive, Image as ImageIcon, Video as VideoIcon } from 'lucide-react'
import { ModelCombobox } from './ModelCombobox'
import type { ModelOptionItem } from './ModelCombobox'
import { useEffect, useMemo, useState } from 'react'
import { settingsApi } from '@/api'
import type { AppSettings, LLMProfile, MCPServiceConfig, MediaProfileConfig } from '@/types'

interface SettingsDialogProps {
  open: boolean
  settings: AppSettings | null
  onClose: () => void
  onSave: (settings: AppSettings) => Promise<void> | void
}

const emptyProfile = (): LLMProfile => ({
  id: `profile-${Date.now()}`,
  name: '新的模型服务',
  base_url: 'http://127.0.0.1:8777/v1',
  api_key: '',
  api_keys: [],
  models: [],
  default_model: '',
})

const emptyMediaProfile = (kind: 'image' | 'video'): MediaProfileConfig => ({
  id: `profile-${Date.now()}`,
  name: kind === 'image' ? '新的图片模型服务' : '新的视频模型服务',
  base_url: '',
  api_key: '',
  api_keys: [],
  models: [],
  model: '',
  default_model: '',
})

/// 多配置列表兼容旧单字段：未升级的存量设置自动包装成单元素列表
function getMediaProfiles(draft: AppSettings, kind: 'image' | 'video'): MediaProfileConfig[] {
  const list = kind === 'image' ? draft.image_profiles : draft.video_profiles
  if (Array.isArray(list) && list.length > 0) return list
  const legacy = kind === 'image' ? draft.image_profile : draft.video_profile
  if (legacy && (legacy.base_url || legacy.model || legacy.api_key)) {
    const profile: MediaProfileConfig = {
      id: legacy.id || 'default',
      name: legacy.name || (kind === 'image' ? '默认图片模型服务' : '默认视频模型服务'),
      base_url: legacy.base_url || '',
      api_keys: legacy.api_keys || [],
      api_key: legacy.api_key || '',
      models: legacy.models && legacy.models.length > 0 ? legacy.models : legacy.model ? [legacy.model] : [],
      model: legacy.model || '',
      default_model: legacy.default_model || legacy.model || '',
      has_api_key: legacy.has_api_key,
    }
    return [profile]
  }
  return []
}

function getActiveMediaId(draft: AppSettings, kind: 'image' | 'video'): string {
  const activeId = kind === 'image' ? draft.active_image_profile_id : draft.active_video_profile_id
  if (activeId) return activeId
  const profiles = getMediaProfiles(draft, kind)
  return profiles[0]?.id || ''
}

function getMediaKeys(profile: MediaProfileConfig) {
  const keys = Array.isArray(profile.api_keys) ? profile.api_keys : []
  const merged = [...keys, profile.api_key || '']
    .map((item) => item.trim())
    .filter(Boolean)
  return Array.from(new Set(merged))
}

function getProfileKeys(profile: LLMProfile) {
  const keys = Array.isArray(profile.api_keys) ? profile.api_keys : []
  const merged = [...keys, profile.api_key || '']
    .map((item) => item.trim())
    .filter(Boolean)
  return Array.from(new Set(merged))
}

const emptyMcpServer = (): MCPServiceConfig => ({
  id: `mcp-${Date.now()}`,
  name: '新的 MCP 服务',
  transport: 'http',
  endpoint: 'http://127.0.0.1:3001',
  enabled: true,
  description: '',
})

export function SettingsDialog({ open, settings, onClose, onSave }: SettingsDialogProps) {
  const [section, setSection] = useState<'llm' | 'mcp' | 'search' | 'nas' | 'image' | 'video'>('llm')
  const [draft, setDraft] = useState<AppSettings | null>(settings)
  const [saving, setSaving] = useState(false)
  const [testingMcpId, setTestingMcpId] = useState<string | null>(null)
  const [mcpTestResults, setMcpTestResults] = useState<Record<string, { ok: boolean; message: string; tools: string[] }>>({})
  const [testingNas, setTestingNas] = useState(false)
  const [nasTestResult, setNasTestResult] = useState<{ ok: boolean; message: string } | null>(null)
  const [testingMedia, setTestingMedia] = useState<'image' | 'video' | null>(null)
  const [mediaTestResult, setMediaTestResult] = useState<Record<'image' | 'video', { ok: boolean; message: string } | null>>({ image: null, video: null })
  const [testingLlmProfile, setTestingLlmProfile] = useState<string | null>(null)
  const [llmTestResult, setLlmTestResult] = useState<Record<string, { ok: boolean; message: string } | null>>({})
  const [fetchingModelProfile, setFetchingModelProfile] = useState<string | null>(null)
  const [fetchModelError, setFetchModelError] = useState('')
  const [fetchedModelsMap, setFetchedModelsMap] = useState<Record<string, ModelOptionItem[]>>({})
  // 媒体（图片/视频）模型的拉取状态与缓存（key = kind:profileId）
  const [fetchedMediaModels, setFetchedMediaModels] = useState<Record<string, ModelOptionItem[]>>({})
  const [fetchingMediaKey, setFetchingMediaKey] = useState<string | null>(null)
  const [mediaFetchError, setMediaFetchError] = useState('')

  useEffect(() => {
    setDraft(settings)
  }, [settings])

  const hasChanges = useMemo(() => JSON.stringify(draft) !== JSON.stringify(settings), [draft, settings])

  // 有 open 但 draft 未加载（settings 为 null）时，显示加载占位而非空白
  if (!open) return null

  if (!draft) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="flex items-center gap-2 text-surface-400">
          <Loader2 className="h-5 w-5 animate-spin" />
          <span>加载设置中…</span>
        </div>
      </div>
    )
  }

  const updateDraft = (patch: Partial<AppSettings>) => {
    setDraft((prev) => (prev ? { ...prev, ...patch } : prev))
  }

  const updateProfile = (id: string, patch: Partial<LLMProfile>) => {
    const nextProfiles = draft.llm_profiles.map((profile) => {
      if (profile.id !== id) return profile
      const next = { ...profile, ...patch }
      if (!next.models.includes(next.default_model)) {
        next.default_model = next.models[0] || ''
      }
      return next
    })
    updateDraft({ llm_profiles: nextProfiles })
  }

  const removeProfile = (id: string) => {
    if (draft.llm_profiles.length <= 1) return
    const nextProfiles = draft.llm_profiles.filter((profile) => profile.id !== id)
    const activeProfile = nextProfiles.find((profile) => profile.id === draft.active_profile_id) || nextProfiles[0]
    updateDraft({
      llm_profiles: nextProfiles,
      active_profile_id: activeProfile.id,
      default_model: activeProfile.default_model,
      active_model: activeProfile.default_model,
    })
  }

  const addProfile = () => {
    const profile = emptyProfile()
    updateDraft({
      llm_profiles: [...draft.llm_profiles, profile],
      active_profile_id: profile.id,
      default_model: profile.default_model,
      active_model: profile.default_model,
    })
  }

  const setActiveProfile = (profileId: string) => {
    const profile = draft.llm_profiles.find((item) => item.id === profileId)
    if (!profile) return
    updateDraft({
      active_profile_id: profileId,
      default_model: profile.default_model,
      active_model: profile.default_model,
    })
  }

  const updateMcp = (id: string, patch: Partial<MCPServiceConfig>) => {
    updateDraft({
      mcp_servers: draft.mcp_servers.map((server) => (server.id === id ? { ...server, ...patch } : server)),
    })
  }

  const addMcp = () => updateDraft({ mcp_servers: [...draft.mcp_servers, emptyMcpServer()] })

  const removeMcp = (id: string) => updateDraft({ mcp_servers: draft.mcp_servers.filter((server) => server.id !== id) })

  // ── 图片/视频多配置（与推理模型一致的「多配置随时切换」） ──
  const updateMediaProfile = (kind: 'image' | 'video', id: string, patch: Partial<MediaProfileConfig>) => {
    const listKey = kind === 'image' ? 'image_profiles' : 'video_profiles'
    const activeKey = kind === 'image' ? 'active_image_profile_id' : 'active_video_profile_id'
    const profiles = getMediaProfiles(draft, kind)
    const nextProfiles = profiles.map((profile) => {
      if (profile.id !== id) return profile
      const next = { ...profile, ...patch }
      // model 是默认模型（单模型为主，models 同步）
      const models = next.models && next.models.length > 0 ? next.models : next.model ? [next.model] : []
      if (!next.model) next.model = models[0] || ''
      return { ...next, models }
    })
    const patchObj: Partial<AppSettings> = { [listKey]: nextProfiles } as Partial<AppSettings>
    const legacyKey = kind === 'image' ? 'image_profile' : 'video_profile'
    if (nextProfiles.length > 0 && getActiveMediaId(draft, kind) === id) {
      patchObj[legacyKey as 'image_profile'] = nextProfiles.find((p) => p.id === id) as any
    }
    updateDraft(patchObj)
  }

  const setActiveMediaProfile = (kind: 'image' | 'video', id: string) => {
    const activeKey = kind === 'image' ? 'active_image_profile_id' : 'active_video_profile_id'
    const legacyKey = kind === 'image' ? 'image_profile' : 'video_profile'
    const profiles = getMediaProfiles(draft, kind)
    const profile = profiles.find((p) => p.id === id)
    if (!profile) return
    updateDraft({ [activeKey]: id, [legacyKey]: profile } as Partial<AppSettings>)
  }

  const addMediaProfile = (kind: 'image' | 'video') => {
    const listKey = kind === 'image' ? 'image_profiles' : 'video_profiles'
    const activeKey = kind === 'image' ? 'active_image_profile_id' : 'active_video_profile_id'
    const legacyKey = kind === 'image' ? 'image_profile' : 'video_profile'
    const profiles = getMediaProfiles(draft, kind)
    const profile = emptyMediaProfile(kind)
    updateDraft({
      [listKey]: [...profiles, profile],
      [activeKey]: profile.id,
      [legacyKey]: profile,
    } as Partial<AppSettings>)
  }

  const removeMediaProfile = (kind: 'image' | 'video', id: string) => {
    const listKey = kind === 'image' ? 'image_profiles' : 'video_profiles'
    const activeKey = kind === 'image' ? 'active_image_profile_id' : 'active_video_profile_id'
    const legacyKey = kind === 'image' ? 'image_profile' : 'video_profile'
    const profiles = getMediaProfiles(draft, kind)
    if (profiles.length <= 1) return
    const nextProfiles = profiles.filter((profile) => profile.id !== id)
    const activeProfile = nextProfiles.find((profile) => profile.id === getActiveMediaId(draft, kind)) || nextProfiles[0]
    updateDraft({
      [listKey]: nextProfiles,
      [activeKey]: activeProfile.id,
      [legacyKey]: activeProfile,
    } as Partial<AppSettings>)
  }

  // 拉取真实模型列表
  const handleFetchModels = async (profileId: string) => {
    const profile = draft.llm_profiles.find((p) => p.id === profileId)
    if (!profile) return
    setFetchingModelProfile(profileId)
    setFetchModelError('')
    try {
      const keys = getProfileKeys(profile)
      const apiKey = keys[0] || ''
      const { data } = await settingsApi.fetchModels(profile.base_url, apiKey)
      if (data.models && data.models.length > 0) {
        // 能力标记：不具备工具调用（或属于生图/生视频）的模型仅列名、不可勾选
        const items: ModelOptionItem[] = (data.models as any[]).map((m: any) => {
          const id = typeof m === 'string' ? m : m.id
          const image = !!(typeof m === 'object' && m.image)
          const video = !!(typeof m === 'object' && m.video)
          const fc = typeof m === 'object' ? !!m.fc : true
          return {
            id,
            disabled: !fc,
            badge: !fc ? (image ? '生图模型' : video ? '生视频模型' : '不支持工具调用') : undefined,
          }
        })
        setFetchedModelsMap((prev) => ({ ...prev, [profileId]: items }))
        const capable = items.filter((i) => !i.disabled).map((i) => i.id)
        if (capable.length > 0) {
          updateProfile(profileId, { models: capable, default_model: capable[0] })
        } else {
          setFetchModelError('该服务没有具备工具调用能力的推理模型')
        }
      } else {
        setFetchModelError('未拉取到模型，请检查 Base URL 和 API Key')
      }
    } catch (err: any) {
      setFetchModelError(err.response?.data?.detail || '拉取模型列表失败')
    } finally {
      setFetchingModelProfile(null)
    }
  }

  const fetchMediaModels = async (kind: 'image' | 'video', profile: MediaProfileConfig) => {
    const mediaKey = kind + ':' + profile.id
    setFetchingMediaKey(mediaKey)
    setMediaFetchError('')
    try {
      const keys = getMediaKeys(profile)
      const apiKey = keys[0] || ''
      const { data } = await settingsApi.fetchModels(profile.base_url, apiKey)
      if (data.models && data.models.length > 0) {
        const items: ModelOptionItem[] = (data.models as any[]).map((m: any) => {
          const id = typeof m === 'string' ? m : m.id
          const capable = kind === 'image' ? !!(typeof m === 'object' && m.image) : !!(typeof m === 'object' && m.video)
          return {
            id,
            disabled: !capable,
            badge: capable ? undefined : kind === 'image' ? '非生图模型' : '非生视频模型',
          }
        })
        setFetchedMediaModels((prev) => ({ ...prev, [mediaKey]: items }))
      } else {
        setMediaFetchError('未拉取到模型，请检查 Base URL 和 API Key')
      }
    } catch (err: any) {
      setMediaFetchError(err.response?.data?.detail || '拉取模型列表失败')
    } finally {
      setFetchingMediaKey(null)
    }
  }

  const testMcp = async (server: MCPServiceConfig) => {
    setTestingMcpId(server.id)
    try {
      const res = await settingsApi.testMcp(server)
      const tools = Array.isArray(res.data?.tools)
        ? res.data.tools.map((item: any) => item?.name).filter(Boolean)
        : []
      setMcpTestResults((prev) => ({
        ...prev,
        [server.id]: {
          ok: !!res.data?.ok,
          message: res.data?.message || '测试完成',
          tools,
        },
      }))
    } catch (err: any) {
      setMcpTestResults((prev) => ({
        ...prev,
        [server.id]: {
          ok: false,
          message: err.response?.data?.detail || err.message || '测试失败',
          tools: [],
        },
      }))
    } finally {
      setTestingMcpId(null)
    }
  }

  const testNas = async () => {
    if (!draft?.nas_config) return
    setTestingNas(true)
    setNasTestResult(null)
    try {
      const res = await settingsApi.testNas(draft.nas_config)
      setNasTestResult({
        ok: !!res.data?.ok,
        message: res.data?.message || '测试完成',
      })
    } catch (err: any) {
      setNasTestResult({
        ok: false,
        message: err.response?.data?.detail || err.message || '测试失败',
      })
    } finally {
      setTestingNas(false)
    }
  }

  const testMedia = async (kind: 'image' | 'video') => {
    const profiles = getMediaProfiles(draft!, kind)
    const activeId = getActiveMediaId(draft!, kind)
    const profile = profiles.find((p) => p.id === activeId) || profiles[0]
    if (!profile) return
    const keys = getMediaKeys(profile)
    setTestingMedia(kind)
    setMediaTestResult((prev) => ({ ...prev, [kind]: null }))
    try {
      const res = await settingsApi.testLlm({ kind, base_url: profile.base_url, api_key: keys[0] || profile.api_key || '', model: profile.model || profile.default_model })
      setMediaTestResult((prev) => ({
        ...prev,
        [kind]: { ok: !!res.data?.ok, message: res.data?.message || '检测完成' },
      }))
    } catch (err: any) {
      setMediaTestResult((prev) => ({
        ...prev,
        [kind]: { ok: false, message: err.response?.data?.detail || err.message || '检测失败' },
      }))
    } finally {
      setTestingMedia(null)
    }
  }

  const testLlmCapability = async (profile: LLMProfile) => {
    const keys = getProfileKeys(profile)
    const apiKey = keys[0] || ''
    const model = profile.models?.[0] || profile.default_model || ''
    setTestingLlmProfile(profile.id)
    setLlmTestResult((prev) => ({ ...prev, [profile.id]: null }))
    try {
      const res = await settingsApi.testLlm({ kind: 'text', base_url: profile.base_url, api_key: apiKey, model })
      setLlmTestResult((prev) => ({
        ...prev,
        [profile.id]: { ok: !!res.data?.ok, message: res.data?.message || '检测完成' },
      }))
    } catch (err: any) {
      setLlmTestResult((prev) => ({
        ...prev,
        [profile.id]: { ok: false, message: err.response?.data?.detail || err.message || '检测失败' },
      }))
    } finally {
      setTestingLlmProfile(null)
    }
  }

  const handleSave = async () => {
    if (!draft) return
    setSaving(true)
    try {
      await onSave({
        ...draft,
        updated_at: new Date().toISOString(),
      })
    } finally {
      setSaving(false)
    }
  }

  const activeProfile = draft.llm_profiles.find((profile) => profile.id === draft.active_profile_id) || draft.llm_profiles[0]
  const imageProfiles = getMediaProfiles(draft, 'image')
  const videoProfiles = getMediaProfiles(draft, 'video')
  const activeImageId = getActiveMediaId(draft, 'image')
  const activeVideoId = getActiveMediaId(draft, 'video')
  const activeImageProfile = imageProfiles.find((p) => p.id === activeImageId) || imageProfiles[0]
  const activeVideoProfile = videoProfiles.find((p) => p.id === activeVideoId) || videoProfiles[0]

  return (
    <div className="h-full overflow-hidden bg-transparent p-5">
      <div className="mx-auto flex h-full max-w-6xl overflow-hidden rounded-[2rem] border border-black/[0.06] bg-white/72 shadow-[0_24px_80px_rgba(24,24,27,0.10)] backdrop-blur-2xl">
        <aside className="w-56 shrink-0 border-r border-black/[0.06] bg-[#eee9df]/70 p-4">
          <div className="mb-5 flex items-center justify-between">
            <div>
              <div className="text-lg font-bold tracking-tight text-surface-950">设置</div>
            </div>
            <button onClick={onClose} className="rounded-full bg-white/75 p-2 text-surface-500 hover:bg-white hover:text-surface-950" title="关闭">
              <X className="h-4 w-4" />
            </button>
          </div>

          <div className="space-y-1.5">
            {[
              ['llm', '模型服务', Cpu],
              ['image', '图片模型', ImageIcon],
              ['video', '视频模型', VideoIcon],
              ['search', '搜索服务', Search],
              ['nas', 'WebDAV 数据源', HardDrive],
              ['mcp', 'MCP 服务', KeyRound],
            ].map(([key, label, Icon]: any) => (
              <button
                key={key}
                type="button"
                onClick={() => setSection(key)}
                className={`flex w-full items-center gap-2 rounded-2xl px-3 py-2.5 text-left text-sm font-semibold transition-all ${section === key ? 'bg-surface-950 text-white shadow-sm' : 'text-surface-600 hover:bg-white/75 hover:text-surface-950'}`}
              >
                <Icon className="h-4 w-4" />
                {label}
              </button>
            ))}
          </div>
        </aside>

        <main className="min-w-0 flex-1 overflow-y-auto p-6">
          {section === 'llm' && (
            <div>
              <div className="mb-5 flex items-start justify-between gap-4">
                <div>
                  <h2 className="text-xl font-bold tracking-tight text-surface-950">模型服务</h2>
                  <p className="mt-1 text-sm text-surface-500">配置多个模型服务，选择默认启用的模型。</p>
                </div>
                <button onClick={addProfile} className="inline-flex items-center gap-1.5 rounded-full bg-surface-950 px-4 py-2 text-sm font-semibold text-white hover:bg-surface-800">
                  <Plus className="h-4 w-4" />
                  添加配置
                </button>
              </div>

              {fetchModelError && (
                <div className="mb-4 rounded-lg bg-red-50 px-4 py-2.5 text-sm text-red-600">{fetchModelError}</div>
              )}

              <div className="mb-5 rounded-[1.5rem] border border-black/[0.06] bg-[#f8f5ee]/80 p-4">
                <label className="mb-2 block text-xs font-bold uppercase tracking-[0.14em] text-surface-500">当前默认模型</label>
                <div className="grid gap-3 md:grid-cols-2">
                  <select
                    value={draft.active_profile_id}
                    onChange={(event) => setActiveProfile(event.target.value)}
                    className="rounded-2xl border border-black/10 bg-white px-3 py-2.5 text-sm outline-none focus:border-surface-500"
                  >
                    {draft.llm_profiles.map((profile) => (
                      <option key={profile.id} value={profile.id}>{profile.name}</option>
                    ))}
                  </select>
                  <select
                    value={draft.active_model}
                    onChange={(event) => updateDraft({ active_model: event.target.value, default_model: event.target.value })}
                    className="rounded-2xl border border-black/10 bg-white px-3 py-2.5 text-sm outline-none focus:border-surface-500"
                  >
                    {(activeProfile?.models || []).map((model) => (
                      <option key={model} value={model}>{model}</option>
                    ))}
                  </select>
                </div>
              </div>

              <div className="grid gap-4 xl:grid-cols-2">
                {draft.llm_profiles.map((profile) => {
                  const isActive = draft.active_profile_id === profile.id
                  return (
                    <div key={profile.id} className={`rounded-[1.6rem] border p-4 shadow-sm transition-all ${isActive ? 'border-surface-900 bg-white ring-2 ring-surface-950/5' : 'border-black/[0.06] bg-white/75'}`}>
                      <div className="mb-4 flex items-start justify-between gap-3">
                        <div className="min-w-0 flex-1">
                          <input
                            value={profile.name}
                            onChange={(event) => updateProfile(profile.id, { name: event.target.value })}
                            className="w-full bg-transparent text-base font-bold text-surface-950 outline-none"
                            placeholder="配置名称"
                          />
                          <div className="mt-1 flex items-center gap-2 text-[11px] text-surface-400">
                            <KeyRound className="h-3 w-3" />
                            Key 池 {getProfileKeys(profile).length} 个
                          </div>
                        </div>
                        <div className="flex items-center gap-1.5">
                          {isActive ? (
                            <span className="inline-flex items-center gap-1 rounded-full bg-surface-950 px-2.5 py-1 text-[10px] font-bold text-white">
                              <Check className="h-3 w-3" />启用中
                            </span>
                          ) : (
                            <button onClick={() => setActiveProfile(profile.id)} className="rounded-full border border-black/10 bg-white px-3 py-1 text-xs font-semibold text-surface-600 hover:bg-surface-50">启用</button>
                          )}
                          <button
                            onClick={() => removeProfile(profile.id)}
                            disabled={draft.llm_profiles.length <= 1}
                            className="rounded-full p-2 text-surface-400 hover:bg-red-50 hover:text-red-600 disabled:opacity-30"
                            title="删除配置"
                          >
                            <Trash2 className="h-4 w-4" />
                          </button>
                        </div>
                      </div>

                      <div className="space-y-3">
                        <label className="block text-xs font-semibold text-surface-500">
                          Base URL
                          <input
                            value={profile.base_url}
                            onChange={(event) => updateProfile(profile.id, { base_url: event.target.value })}
                            className="mt-1.5 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 text-sm text-surface-900 outline-none focus:border-surface-500"
                          />
                        </label>
                        <label className="block text-xs font-semibold text-surface-500">
                          API Key 池（每行一个，按请求轮询负载）
                          <textarea
                            value={getProfileKeys(profile).join('\n')}
                            onChange={(event) => {
                              const apiKeys = event.target.value.split('\n').map((item) => item.trim()).filter(Boolean)
                              updateProfile(profile.id, { api_keys: apiKeys, api_key: '', has_api_key: apiKeys.length > 0 })
                            }}
                            rows={4}
                            placeholder="sk-..."
                            className="mt-1.5 w-full resize-y rounded-2xl border border-black/10 bg-white px-3 py-2.5 font-mono text-xs text-surface-900 outline-none focus:border-surface-500"
                          />
                        </label>
                        <label className="block text-xs font-semibold text-surface-500">
                          <span className="mb-1.5 block">模型列表</span>
                          <ModelCombobox
                            models={profile.models || []}
                            options={fetchedModelsMap[profile.id] || []}
                            loading={fetchingModelProfile === profile.id}
                            onChange={(models) => updateProfile(profile.id, { models, default_model: models[0] || profile.default_model })}
                            onFetch={() => handleFetchModels(profile.id)}
                          />
                          <span className="mt-1 block text-[11px] text-surface-400">
                            点开选择已有模型，或输入新模型名回车添加；「拉取」从 Base URL 获取真实列表。
                          </span>
                        </label>

                        <button
                          type="button"
                          onClick={() => testLlmCapability(profile)}
                          disabled={testingLlmProfile === profile.id}
                          className="flex items-center gap-2 rounded-2xl border border-surface-300 bg-white px-4 py-2 text-sm font-semibold text-surface-700 transition hover:bg-surface-50 disabled:cursor-not-allowed disabled:opacity-50"
                        >
                          {testingLlmProfile === profile.id ? '检测中...' : '检测能力（含工具调用）'}
                        </button>

                        {llmTestResult[profile.id] && (
                          <div className={`rounded-lg px-3 py-2 text-sm break-all whitespace-pre-wrap leading-relaxed ${llmTestResult[profile.id]!.ok ? 'bg-emerald-50 text-emerald-700' : 'bg-red-50 text-red-600'}`}>
                            {llmTestResult[profile.id]!.message}
                          </div>
                        )}
                      </div>
                    </div>
                  )
                })}
              </div>
            </div>
          )}

          {section === 'search' && (
            <div>
              <h2 className="text-xl font-bold tracking-tight text-surface-950">搜索服务</h2>
              <p className="mt-1 text-sm text-surface-500">配置联网搜索的 API Key（每个用户各自的 key，保存在服务器端）。</p>
              <div className="mt-5 space-y-4">
                <label className="block text-sm font-semibold text-surface-600">
                  优先搜索源
                  <select
                    value={draft.search_providers?.provider || 'auto'}
                    onChange={(event) => updateDraft({ search_providers: { ...(draft.search_providers || { tavily_api_key: '', brave_api_key: '', kimi_api_key: '', provider: 'auto' }), provider: event.target.value } })}
                    className="mt-2 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 text-sm outline-none focus:border-surface-500"
                  >
                    <option value="auto">自动（优先有 key 的源）</option>
                    <option value="tavily">Tavily</option>
                    <option value="brave">Brave</option>
                    <option value="kimi">Kimi</option>
                    <option value="duckduckgo">DuckDuckGo（免费）</option>
                  </select>
                </label>

                <label className="block text-sm font-semibold text-surface-600">
                  Tavily API Key
                  <input
                    type="password"
                    value={draft.search_providers?.tavily_api_key || ''}
                    onChange={(event) => updateDraft({ search_providers: { ...(draft.search_providers || { brave_api_key: '', kimi_api_key: '', provider: 'auto' }), tavily_api_key: event.target.value } })}
                    placeholder="tvly-..."
                    className="mt-2 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 font-mono text-sm outline-none focus:border-surface-500"
                  />
                </label>

                <label className="block text-sm font-semibold text-surface-600">
                  Brave Search API Key
                  <input
                    type="password"
                    value={draft.search_providers?.brave_api_key || ''}
                    onChange={(event) => updateDraft({ search_providers: { ...(draft.search_providers || { tavily_api_key: '', kimi_api_key: '', provider: 'auto' }), brave_api_key: event.target.value } })}
                    placeholder="BSA..."
                    className="mt-2 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 font-mono text-sm outline-none focus:border-surface-500"
                  />
                </label>

                <label className="block text-sm font-semibold text-surface-600">
                  Kimi API Key
                  <input
                    type="password"
                    value={draft.search_providers?.kimi_api_key || ''}
                    onChange={(event) => updateDraft({ search_providers: { ...(draft.search_providers || { tavily_api_key: '', brave_api_key: '', provider: 'auto' }), kimi_api_key: event.target.value } })}
                    placeholder="sk-..."
                    className="mt-2 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 font-mono text-sm outline-none focus:border-surface-500"
                  />
                </label>

                <p className="rounded-lg bg-surface-50 px-3 py-2 text-xs text-surface-400">
                  DuckDuckGo 免费无需 key；Tavily / Brave / Kimi 需要各自的 API Key。未填 key 的源会自动跳过。
                </p>
              </div>
            </div>
          )}

          {section === 'nas' && (
            <div>
              <h2 className="text-xl font-bold tracking-tight text-surface-950">WebDAV 数据源</h2>
              <p className="mt-1 text-sm text-surface-500">通过 HTTP(S) WebDAV 协议直接访问懒猫微服 NAS 文件，不在文件系统上挂载。每个用户填各自的懒猫账号 WebDAV 凭据，懒猫微服按账号隔离文件空间。</p>
              <div className="mt-5 space-y-4">
                <label className="flex items-center gap-2 text-sm font-semibold text-surface-600">
                  <input
                    type="checkbox"
                    checked={draft.nas_config?.enabled || false}
                    onChange={(event) => updateDraft({ nas_config: { ...(draft.nas_config || { name: '', base_url: '', username: '', password: '', enabled: false }), enabled: event.target.checked } })}
                    className="h-4 w-4 rounded border-surface-300 text-surface-950"
                  />
                  启用 WebDAV 数据源
                </label>

                <label className="block text-sm font-semibold text-surface-600">
                  数据源名称
                  <input
                    value={draft.nas_config?.name || ''}
                    onChange={(event) => updateDraft({ nas_config: { ...(draft.nas_config || { base_url: '', username: '', password: '', enabled: true }), name: event.target.value } })}
                    placeholder="如：公司资料库"
                    className="mt-2 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 text-sm outline-none focus:border-surface-500"
                  />
                </label>

                <label className="block text-sm font-semibold text-surface-600">
                  WebDAV 地址
                  <input
                    value={draft.nas_config?.base_url || ''}
                    onChange={(event) => updateDraft({ nas_config: { ...(draft.nas_config || { name: '', username: '', password: '', enabled: true }), base_url: event.target.value } })}
                    placeholder="https://xxx.heiyu.space/dav"
                    className="mt-2 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 font-mono text-sm outline-none focus:border-surface-500"
                  />
                </label>

                <div className="grid gap-4 md:grid-cols-2">
                  <label className="block text-sm font-semibold text-surface-600">
                    用户名
                    <input
                      value={draft.nas_config?.username || ''}
                      onChange={(event) => updateDraft({ nas_config: { ...(draft.nas_config || { name: '', base_url: '', password: '', enabled: true }), username: event.target.value } })}
                      className="mt-2 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 text-sm outline-none focus:border-surface-500"
                    />
                  </label>
                  <label className="block text-sm font-semibold text-surface-600">
                    密码
                    <input
                      type="password"
                      value={draft.nas_config?.password || ''}
                      onChange={(event) => updateDraft({ nas_config: { ...(draft.nas_config || { name: '', base_url: '', username: '', enabled: true }), password: event.target.value } })}
                      className="mt-2 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 font-mono text-sm outline-none focus:border-surface-500"
                    />
                  </label>
                </div>

                <button
                  type="button"
                  onClick={testNas}
                  disabled={testingNas}
                  className="flex items-center gap-2 rounded-2xl border border-surface-300 bg-white px-4 py-2 text-sm font-semibold text-surface-700 transition hover:bg-surface-50 disabled:cursor-not-allowed disabled:opacity-50"
                >
                  {testingNas ? '测试中...' : '测试连接'}
                </button>

                {nasTestResult && (
                  <div className={`rounded-lg px-3 py-2 text-sm break-all whitespace-pre-wrap leading-relaxed ${nasTestResult.ok ? 'bg-emerald-50 text-emerald-700' : 'bg-red-50 text-red-600'}`}>
                    {nasTestResult.message}
                  </div>
                )}

                <p className="rounded-lg bg-surface-50 px-3 py-2 text-xs text-surface-400">
                  懒猫微服 WebDAV 通过 HTTP(S) 协议直接访问，无需在文件系统挂载。每个懒猫账号的 WebDAV 用户名/密码对应其自己的文件空间（用户文稿目录），因此不同用户填各自的凭据即天然隔离，无需额外配置目录路径。
                </p>
              </div>
            </div>
          )}

          {section === 'image' && (
            <div>
              <div className="mb-5 flex items-start justify-between gap-4">
                <div>
                  <h2 className="text-xl font-bold tracking-tight text-surface-950">图片模型</h2>
                  <p className="mt-1 text-sm text-surface-500">保存多个生图模型服务，随时切换启用；未配置时回退到环境变量。</p>
                </div>
                <button onClick={() => addMediaProfile('image')} className="inline-flex items-center gap-1.5 rounded-full bg-surface-950 px-4 py-2 text-sm font-semibold text-white hover:bg-surface-800">
                  <Plus className="h-4 w-4" />
                  添加配置
                </button>
              </div>

              {mediaFetchError && (
                <div className="mb-4 rounded-lg bg-red-50 px-4 py-2.5 text-sm text-red-600">{mediaFetchError}</div>
              )}

              {imageProfiles.length > 0 && (
                <div className="mb-5 rounded-[1.5rem] border border-black/[0.06] bg-[#f8f5ee]/80 p-4">
                  <label className="mb-2 block text-xs font-bold uppercase tracking-[0.14em] text-surface-500">当前启用配置</label>
                  <div className="grid gap-3 md:grid-cols-2">
                    <select
                      value={activeImageId}
                      onChange={(event) => setActiveMediaProfile('image', event.target.value)}
                      className="rounded-2xl border border-black/10 bg-white px-3 py-2.5 text-sm outline-none focus:border-surface-500"
                    >
                      {imageProfiles.map((profile) => (
                        <option key={profile.id} value={profile.id}>{profile.name}</option>
                      ))}
                    </select>
                    <div className="flex items-center gap-2">
                      <select
                        value={activeImageProfile?.model || ''}
                        onChange={(event) => activeImageProfile && updateMediaProfile('image', activeImageProfile.id, { model: event.target.value, models: [event.target.value], default_model: event.target.value })}
                        className="min-w-0 flex-1 rounded-2xl border border-black/10 bg-white px-3 py-2.5 font-mono text-sm outline-none focus:border-surface-500"
                      >
                        {(fetchedMediaModels['image:' + activeImageId] && fetchedMediaModels['image:' + activeImageId].length > 0
                          ? fetchedMediaModels['image:' + activeImageId].filter((i) => !i.disabled).map((i) => i.id)
                          : (activeImageProfile?.models && activeImageProfile.models.length > 0 ? activeImageProfile.models : [''])
                        ).map((m) => (
                          <option key={m} value={m}>{m}</option>
                        ))}
                      </select>
                      <button
                        type="button"
                        onClick={() => activeImageProfile && testMedia('image')}
                        disabled={testingMedia === 'image'}
                        className="shrink-0 rounded-2xl border border-surface-300 bg-white px-4 py-2.5 text-sm font-semibold text-surface-700 transition hover:bg-surface-50 disabled:cursor-not-allowed disabled:opacity-50"
                      >
                        {testingMedia === 'image' ? '检测中...' : '检测能力'}
                      </button>
                    </div>
                  </div>
                  {mediaTestResult.image && (
                    <div className={`mt-3 rounded-lg px-3 py-2 text-sm break-all whitespace-pre-wrap leading-relaxed ${mediaTestResult.image.ok ? 'bg-emerald-50 text-emerald-700' : 'bg-red-50 text-red-600'}`}>
                      {mediaTestResult.image.message}
                    </div>
                  )}
                </div>
              )}

              <div className="grid gap-4 xl:grid-cols-2">
                {imageProfiles.map((profile) => {
                  const isActive = activeImageId === profile.id
                  return (
                    <div key={profile.id} className={`rounded-[1.6rem] border p-4 shadow-sm transition-all ${isActive ? 'border-surface-900 bg-white ring-2 ring-surface-950/5' : 'border-black/[0.06] bg-white/75'}`}>
                      <div className="mb-4 flex items-start justify-between gap-3">
                        <div className="min-w-0 flex-1">
                          <input
                            value={profile.name}
                            onChange={(event) => updateMediaProfile('image', profile.id, { name: event.target.value })}
                            className="w-full bg-transparent text-base font-bold text-surface-950 outline-none"
                            placeholder="配置名称"
                          />
                          <div className="mt-1 flex items-center gap-2 text-[11px] text-surface-400">
                            <KeyRound className="h-3 w-3" />
                            Key 池 {getMediaKeys(profile).length} 个
                          </div>
                        </div>
                        <div className="flex items-center gap-1.5">
                          {isActive ? (
                            <span className="inline-flex items-center gap-1 rounded-full bg-surface-950 px-2.5 py-1 text-[10px] font-bold text-white">
                              <Check className="h-3 w-3" />启用中
                            </span>
                          ) : (
                            <button onClick={() => setActiveMediaProfile('image', profile.id)} className="rounded-full border border-black/10 bg-white px-3 py-1 text-xs font-semibold text-surface-600 hover:bg-surface-50">启用</button>
                          )}
                          <button
                            onClick={() => removeMediaProfile('image', profile.id)}
                            disabled={imageProfiles.length <= 1}
                            className="rounded-full p-2 text-surface-400 hover:bg-red-50 hover:text-red-600 disabled:opacity-30"
                            title="删除配置"
                          >
                            <Trash2 className="h-4 w-4" />
                          </button>
                        </div>
                      </div>

                      <div className="space-y-3">
                        <label className="block text-xs font-semibold text-surface-500">
                          Base URL
                          <input
                            value={profile.base_url}
                            onChange={(event) => updateMediaProfile('image', profile.id, { base_url: event.target.value })}
                            className="mt-1.5 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 font-mono text-sm text-surface-900 outline-none focus:border-surface-500"
                            placeholder="https://apihub.agnes-ai.com"
                          />
                        </label>
                        <label className="block text-xs font-semibold text-surface-500">
                          API Key 池（每行一个，按请求轮询负载）
                          <textarea
                            value={getMediaKeys(profile).join('\n')}
                            onChange={(event) => {
                              const apiKeys = event.target.value.split('\n').map((item) => item.trim()).filter(Boolean)
                              updateMediaProfile('image', profile.id, { api_keys: apiKeys, api_key: '', has_api_key: apiKeys.length > 0 })
                            }}
                            rows={3}
                            placeholder="sk-..."
                            className="mt-1.5 w-full resize-y rounded-2xl border border-black/10 bg-white px-3 py-2.5 font-mono text-xs text-surface-900 outline-none focus:border-surface-500"
                          />
                        </label>
                        <label className="block text-xs font-semibold text-surface-500">
                          <span className="mb-1.5 block">模型（点「拉取」从服务端获取真实列表，支持搜索与滚动）</span>
                          <ModelCombobox
                            models={profile.model ? [profile.model] : []}
                            options={fetchedMediaModels['image:' + profile.id] || []}
                            loading={fetchingMediaKey === 'image:' + profile.id}
                            onChange={(models) => updateMediaProfile('image', profile.id, { model: models[0] || '', models: models.slice(0, 1), default_model: models[0] || '' })}
                            onFetch={() => fetchMediaModels('image', profile)}
                          />
                        </label>

                        {/* 推荐模型快捷选择（火山方舟 Seedream，标注组图能力） */}
                        <div className="rounded-2xl border border-surface-200 bg-surface-50/60 p-3">
                          <div className="mb-2 flex items-center gap-1.5 text-xs font-semibold text-surface-500">
                            <span>推荐模型（火山方舟）</span>
                            <span className="rounded-full bg-amber-100 px-1.5 py-0.5 text-[10px] text-amber-700">支持组图</span>
                          </div>
                          <div className="flex flex-wrap gap-1.5">
                            {[
                              { model: 'doubao-seedream-5-0-260128', label: '5.0 标准', seq: true },
                              { model: 'doubao-seedream-5-0-lite-260128', label: '5.0 Lite', seq: true },
                              { model: 'doubao-seedream-4-5-251128', label: '4.5', seq: true },
                              { model: 'doubao-seedream-5-0-pro-260628', label: '5.0 Pro', seq: false },
                            ].map((item) => {
                              const active = (profile.model || '') === item.model
                              return (
                                <button
                                  key={item.model}
                                  type="button"
                                  onClick={() => updateMediaProfile('image', profile.id, { model: item.model, models: [item.model], default_model: item.model })}
                                  className={`inline-flex items-center gap-1 rounded-lg px-2.5 py-1.5 text-[11px] font-medium transition ${
                                    active
                                      ? 'bg-indigo-500 text-white'
                                      : 'bg-white text-surface-600 border border-surface-200 hover:bg-indigo-50'
                                  }`}
                                >
                                  <span>{item.label}</span>
                                  {item.seq && <span className={`rounded px-1 text-[9px] leading-4 ${active ? 'bg-white/20 text-white' : 'bg-emerald-100 text-emerald-700'}`}>组图</span>}
                                </button>
                              )
                            })}
                          </div>
                          <p className="mt-2 text-[11px] leading-relaxed text-surface-400">
                            标注「组图」的模型支持一次生成多张图（更快）；Pro 仅支持单图，将自动并行生成。选 Pro 需将 Base URL 设为火山方舟地址（如 https://ark.cn-beijing.volces.com/api/v3）。
                          </p>
                        </div>
                      </div>
                    </div>
                  )
                })}
              </div>
            </div>
          )}

          {section === 'video' && (
            <div>
              <div className="mb-5 flex items-start justify-between gap-4">
                <div>
                  <h2 className="text-xl font-bold tracking-tight text-surface-950">视频模型</h2>
                  <p className="mt-1 text-sm text-surface-500">保存多个生视频模型服务，随时切换启用；未配置时回退到环境变量。</p>
                </div>
                <button onClick={() => addMediaProfile('video')} className="inline-flex items-center gap-1.5 rounded-full bg-surface-950 px-4 py-2 text-sm font-semibold text-white hover:bg-surface-800">
                  <Plus className="h-4 w-4" />
                  添加配置
                </button>
              </div>

              {videoProfiles.length > 0 && (
                <div className="mb-5 rounded-[1.5rem] border border-black/[0.06] bg-[#f8f5ee]/80 p-4">
                  <label className="mb-2 block text-xs font-bold uppercase tracking-[0.14em] text-surface-500">当前启用配置</label>
                  <div className="grid gap-3 md:grid-cols-2">
                    <select
                      value={activeVideoId}
                      onChange={(event) => setActiveMediaProfile('video', event.target.value)}
                      className="rounded-2xl border border-black/10 bg-white px-3 py-2.5 text-sm outline-none focus:border-surface-500"
                    >
                      {videoProfiles.map((profile) => (
                        <option key={profile.id} value={profile.id}>{profile.name}</option>
                      ))}
                    </select>
                    <div className="flex items-center gap-2">
                      <select
                        value={activeVideoProfile?.model || ''}
                        onChange={(event) => activeVideoProfile && updateMediaProfile('video', activeVideoProfile.id, { model: event.target.value, models: [event.target.value], default_model: event.target.value })}
                        className="min-w-0 flex-1 rounded-2xl border border-black/10 bg-white px-3 py-2.5 font-mono text-sm outline-none focus:border-surface-500"
                      >
                        {(fetchedMediaModels['video:' + activeVideoId] && fetchedMediaModels['video:' + activeVideoId].length > 0
                          ? fetchedMediaModels['video:' + activeVideoId].filter((i) => !i.disabled).map((i) => i.id)
                          : (activeVideoProfile?.models && activeVideoProfile.models.length > 0 ? activeVideoProfile.models : [''])
                        ).map((m) => (
                          <option key={m} value={m}>{m}</option>
                        ))}
                      </select>
                      <button
                        type="button"
                        onClick={() => activeVideoProfile && testMedia('video')}
                        disabled={testingMedia === 'video'}
                        className="shrink-0 rounded-2xl border border-surface-300 bg-white px-4 py-2.5 text-sm font-semibold text-surface-700 transition hover:bg-surface-50 disabled:cursor-not-allowed disabled:opacity-50"
                      >
                        {testingMedia === 'video' ? '检测中...' : '检测能力'}
                      </button>
                    </div>
                  </div>
                  {mediaTestResult.video && (
                    <div className={`mt-3 rounded-lg px-3 py-2 text-sm break-all whitespace-pre-wrap leading-relaxed ${mediaTestResult.video.ok ? 'bg-emerald-50 text-emerald-700' : 'bg-red-50 text-red-600'}`}>
                      {mediaTestResult.video.message}
                    </div>
                  )}
                </div>
              )}

              <div className="grid gap-4 xl:grid-cols-2">
                {videoProfiles.map((profile) => {
                  const isActive = activeVideoId === profile.id
                  return (
                    <div key={profile.id} className={`rounded-[1.6rem] border p-4 shadow-sm transition-all ${isActive ? 'border-surface-900 bg-white ring-2 ring-surface-950/5' : 'border-black/[0.06] bg-white/75'}`}>
                      <div className="mb-4 flex items-start justify-between gap-3">
                        <div className="min-w-0 flex-1">
                          <input
                            value={profile.name}
                            onChange={(event) => updateMediaProfile('video', profile.id, { name: event.target.value })}
                            className="w-full bg-transparent text-base font-bold text-surface-950 outline-none"
                            placeholder="配置名称"
                          />
                          <div className="mt-1 flex items-center gap-2 text-[11px] text-surface-400">
                            <KeyRound className="h-3 w-3" />
                            Key 池 {getMediaKeys(profile).length} 个
                          </div>
                        </div>
                        <div className="flex items-center gap-1.5">
                          {isActive ? (
                            <span className="inline-flex items-center gap-1 rounded-full bg-surface-950 px-2.5 py-1 text-[10px] font-bold text-white">
                              <Check className="h-3 w-3" />启用中
                            </span>
                          ) : (
                            <button onClick={() => setActiveMediaProfile('video', profile.id)} className="rounded-full border border-black/10 bg-white px-3 py-1 text-xs font-semibold text-surface-600 hover:bg-surface-50">启用</button>
                          )}
                          <button
                            onClick={() => removeMediaProfile('video', profile.id)}
                            disabled={videoProfiles.length <= 1}
                            className="rounded-full p-2 text-surface-400 hover:bg-red-50 hover:text-red-600 disabled:opacity-30"
                            title="删除配置"
                          >
                            <Trash2 className="h-4 w-4" />
                          </button>
                        </div>
                      </div>

                      <div className="space-y-3">
                        <label className="block text-xs font-semibold text-surface-500">
                          Base URL
                          <input
                            value={profile.base_url}
                            onChange={(event) => updateMediaProfile('video', profile.id, { base_url: event.target.value })}
                            className="mt-1.5 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 font-mono text-sm text-surface-900 outline-none focus:border-surface-500"
                            placeholder="https://apihub.agnes-ai.com"
                          />
                        </label>
                        <label className="block text-xs font-semibold text-surface-500">
                          API Key 池（每行一个，按请求轮询负载）
                          <textarea
                            value={getMediaKeys(profile).join('\n')}
                            onChange={(event) => {
                              const apiKeys = event.target.value.split('\n').map((item) => item.trim()).filter(Boolean)
                              updateMediaProfile('video', profile.id, { api_keys: apiKeys, api_key: '', has_api_key: apiKeys.length > 0 })
                            }}
                            rows={3}
                            placeholder="sk-..."
                            className="mt-1.5 w-full resize-y rounded-2xl border border-black/10 bg-white px-3 py-2.5 font-mono text-xs text-surface-900 outline-none focus:border-surface-500"
                          />
                        </label>
                        <label className="block text-xs font-semibold text-surface-500">
                          <span className="mb-1.5 block">模型（点「拉取」从服务端获取真实列表，支持搜索与滚动）</span>
                          <ModelCombobox
                            models={profile.model ? [profile.model] : []}
                            options={fetchedMediaModels['video:' + profile.id] || []}
                            loading={fetchingMediaKey === 'video:' + profile.id}
                            onChange={(models) => updateMediaProfile('video', profile.id, { model: models[0] || '', models: models.slice(0, 1), default_model: models[0] || '' })}
                            onFetch={() => fetchMediaModels('video', profile)}
                          />
                        </label>
                      </div>
                    </div>
                  )
                })}
              </div>
            </div>
          )}

          {section === 'mcp' && (
            <div>
              <div className="mb-5 flex items-start justify-between gap-4">
                <div>
                  <h2 className="text-xl font-bold tracking-tight text-surface-950">MCP 服务</h2>
                  <p className="mt-1 text-sm text-surface-500">保存常用 MCP 服务的地址和启用状态，方便后续接入。</p>
                </div>
                <button onClick={addMcp} className="inline-flex items-center gap-1.5 rounded-full bg-surface-950 px-4 py-2 text-sm font-semibold text-white hover:bg-surface-800">
                  <Plus className="h-4 w-4" />
                  添加服务
                </button>
              </div>

              <div className="space-y-4">
                {draft.mcp_servers.length === 0 && (
                  <div className="rounded-[1.6rem] border border-dashed border-black/10 bg-white/65 px-4 py-8 text-center text-sm text-surface-500">
                    还没有 MCP 服务配置，添加后会保存在当前账号下。
                  </div>
                )}
                {draft.mcp_servers.map((server) => (
                  <div key={server.id} className="rounded-[1.6rem] border border-black/[0.06] bg-white/75 p-4 shadow-sm">
                    <div className="mb-4 flex items-start justify-between gap-3">
                      <div className="min-w-0 flex-1">
                        <input
                          value={server.name}
                          onChange={(event) => updateMcp(server.id, { name: event.target.value })}
                          className="w-full bg-transparent text-base font-bold text-surface-950 outline-none"
                          placeholder="服务名称"
                        />
                        <div className="mt-1 text-[11px] text-surface-400">{server.enabled ? '已启用' : '已停用'}</div>
                      </div>
                      <div className="flex items-center gap-2">
                        <button
                          type="button"
                          onClick={() => testMcp(server)}
                          disabled={testingMcpId === server.id}
                          className="rounded-full border border-black/10 bg-white px-3 py-1 text-xs font-semibold text-surface-600 hover:bg-surface-50 disabled:cursor-not-allowed disabled:opacity-50"
                        >
                          {testingMcpId === server.id ? '测试中...' : '测试连接'}
                        </button>
                        <label className="flex items-center gap-2 text-xs font-semibold text-surface-500">
                          <input
                            type="checkbox"
                            checked={server.enabled}
                            onChange={(event) => updateMcp(server.id, { enabled: event.target.checked })}
                          />
                          启用
                        </label>
                        <button onClick={() => removeMcp(server.id)} className="rounded-full p-2 text-surface-400 hover:bg-red-50 hover:text-red-600" title="删除服务">
                          <Trash2 className="h-4 w-4" />
                        </button>
                      </div>
                    </div>

                    <div className="grid gap-3 md:grid-cols-2">
                      <label className="block text-xs font-semibold text-surface-500">
                        传输方式
                        <select
                          value={server.transport}
                          onChange={(event) => updateMcp(server.id, { transport: event.target.value })}
                          className="mt-1.5 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 text-sm outline-none focus:border-surface-500"
                        >
                          <option value="http">HTTP</option>
                          <option value="sse">SSE</option>
                          <option value="stdio">STDIO</option>
                        </select>
                      </label>
                      <label className="block text-xs font-semibold text-surface-500">
                        服务地址
                        <input
                          value={server.endpoint}
                          onChange={(event) => updateMcp(server.id, { endpoint: event.target.value })}
                          className="mt-1.5 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 text-sm outline-none focus:border-surface-500"
                        />
                      </label>
                      <label className="block text-xs font-semibold text-surface-500 md:col-span-2">
                        描述
                        <input
                          value={server.description || ''}
                          onChange={(event) => updateMcp(server.id, { description: event.target.value })}
                          className="mt-1.5 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 text-sm outline-none focus:border-surface-500"
                        />
                      </label>
                    </div>

                    {mcpTestResults[server.id] && (
                      <div className={`mt-3 rounded-2xl border px-3 py-2 text-xs ${mcpTestResults[server.id].ok ? 'border-emerald-200 bg-emerald-50 text-emerald-700' : 'border-red-200 bg-red-50 text-red-600'}`}>
                        <div className="font-semibold">{mcpTestResults[server.id].message}</div>
                        {mcpTestResults[server.id].tools.length > 0 && (
                          <div className="mt-1 text-[11px]">
                            可用工具：{mcpTestResults[server.id].tools.join(' / ')}
                          </div>
                        )}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            </div>
          )}

          <div className="mt-6 flex items-center justify-end gap-3 border-t border-black/[0.06] pt-5">
            <button onClick={onClose} className="rounded-full border border-black/10 bg-white px-4 py-2 text-sm font-semibold text-surface-600 hover:bg-surface-50">
              取消
            </button>
            <button
              onClick={handleSave}
              disabled={!hasChanges || saving}
              className="rounded-full bg-surface-950 px-5 py-2 text-sm font-semibold text-white disabled:cursor-not-allowed disabled:opacity-40"
            >
              {saving ? '保存中...' : '保存设置'}
            </button>
          </div>
        </main>
      </div>
    </div>
  )
}

import { Check, KeyRound, Plus, Trash2, X, Cpu, LayoutDashboard, Search, Loader2, HardDrive, Image as ImageIcon, Video as VideoIcon } from 'lucide-react'
import { ModelCombobox } from './ModelCombobox'
import { useEffect, useMemo, useState } from 'react'
import { settingsApi } from '@/api'
import type { AppSettings, LLMProfile, MCPServiceConfig } from '@/types'

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
  const [section, setSection] = useState<'llm' | 'base' | 'mcp' | 'search' | 'nas' | 'image' | 'video'>('llm')
  const [draft, setDraft] = useState<AppSettings | null>(settings)
  const [saving, setSaving] = useState(false)
  const [testingMcpId, setTestingMcpId] = useState<string | null>(null)
  const [mcpTestResults, setMcpTestResults] = useState<Record<string, { ok: boolean; message: string; tools: string[] }>>({})
  const [testingNas, setTestingNas] = useState(false)
  const [nasTestResult, setNasTestResult] = useState<{ ok: boolean; message: string } | null>(null)
  const [fetchingModelProfile, setFetchingModelProfile] = useState<string | null>(null)
  const [fetchModelError, setFetchModelError] = useState('')
  const [fetchedModelsMap, setFetchedModelsMap] = useState<Record<string, string[]>>({})

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
        setFetchedModelsMap((prev) => ({ ...prev, [profileId]: data.models }))
        // 自动把拉到的模型合并进已选（首次拉取直接全选，方便快速启用）
        updateProfile(profileId, { models: data.models, default_model: data.models[0] })
      } else {
        setFetchModelError('未拉取到模型，请检查 Base URL 和 API Key')
      }
    } catch (err: any) {
      setFetchModelError(err.response?.data?.detail || '拉取模型列表失败')
    } finally {
      setFetchingModelProfile(null)
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
              ['base', '基础信息', LayoutDashboard],
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
              <h2 className="text-xl font-bold tracking-tight text-surface-950">图片模型</h2>
              <p className="mt-1 text-sm text-surface-500">配置生图（AI 绘画）模型的地址、密钥和模型名。每个用户单独配置，未配置时回退到环境变量。</p>
              <div className="mt-5 space-y-4">
                <label className="block text-sm font-semibold text-surface-600">
                  Base URL
                  <input
                    value={draft.image_profile?.base_url || ''}
                    onChange={(event) => updateDraft({ image_profile: { ...(draft.image_profile || { base_url: '', api_keys: [], api_key: '', model: '' }), base_url: event.target.value } })}
                    placeholder="https://apihub.agnes-ai.com"
                    className="mt-2 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 font-mono text-sm outline-none focus:border-surface-500"
                  />
                </label>
                <label className="block text-sm font-semibold text-surface-600">
                  API Key（多个用逗号分隔，做负载均衡）
                  <input
                    type="password"
                    value={draft.image_profile?.api_key || ''}
                    onChange={(event) => updateDraft({ image_profile: { ...(draft.image_profile || { base_url: '', api_keys: [], model: '' }), api_key: event.target.value } })}
                    placeholder="sk-..."
                    className="mt-2 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 font-mono text-sm outline-none focus:border-surface-500"
                  />
                </label>
                <label className="block text-sm font-semibold text-surface-600">
                  模型名
                  <input
                    value={draft.image_profile?.model || ''}
                    onChange={(event) => updateDraft({ image_profile: { ...(draft.image_profile || { base_url: '', api_keys: [], api_key: '' }), model: event.target.value } })}
                    placeholder="agnes-image-2.1-flash"
                    className="mt-2 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 font-mono text-sm outline-none focus:border-surface-500"
                  />
                </label>
              </div>
            </div>
          )}

          {section === 'video' && (
            <div>
              <h2 className="text-xl font-bold tracking-tight text-surface-950">视频模型</h2>
              <p className="mt-1 text-sm text-surface-500">配置生视频（AI 视频）模型的地址、密钥和模型名。每个用户单独配置，未配置时回退到环境变量。</p>
              <div className="mt-5 space-y-4">
                <label className="block text-sm font-semibold text-surface-600">
                  Base URL
                  <input
                    value={draft.video_profile?.base_url || ''}
                    onChange={(event) => updateDraft({ video_profile: { ...(draft.video_profile || { base_url: '', api_keys: [], api_key: '', model: '' }), base_url: event.target.value } })}
                    placeholder="https://apihub.agnes-ai.com"
                    className="mt-2 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 font-mono text-sm outline-none focus:border-surface-500"
                  />
                </label>
                <label className="block text-sm font-semibold text-surface-600">
                  API Key（多个用逗号分隔，做负载均衡）
                  <input
                    type="password"
                    value={draft.video_profile?.api_key || ''}
                    onChange={(event) => updateDraft({ video_profile: { ...(draft.video_profile || { base_url: '', api_keys: [], model: '' }), api_key: event.target.value } })}
                    placeholder="sk-..."
                    className="mt-2 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 font-mono text-sm outline-none focus:border-surface-500"
                  />
                </label>
                <label className="block text-sm font-semibold text-surface-600">
                  模型名
                  <input
                    value={draft.video_profile?.model || ''}
                    onChange={(event) => updateDraft({ video_profile: { ...(draft.video_profile || { base_url: '', api_keys: [], api_key: '' }), model: event.target.value } })}
                    placeholder="agnes-video-2.5"
                    className="mt-2 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 font-mono text-sm outline-none focus:border-surface-500"
                  />
                </label>
              </div>
            </div>
          )}

          {section === 'base' && (
            <div>
              <h2 className="text-xl font-bold tracking-tight text-surface-950">基础信息</h2>
              <p className="mt-1 text-sm text-surface-500">配置产品名称、工作区标题和默认主题。</p>
              <div className="mt-5 grid gap-4 md:grid-cols-2">
                <label className="block text-sm font-semibold text-surface-600">
                  产品名称
                  <input
                    value={draft.basic.app_name}
                    onChange={(event) => updateDraft({ basic: { ...draft.basic, app_name: event.target.value } })}
                    className="mt-2 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 text-sm outline-none focus:border-surface-500"
                  />
                </label>
                <label className="block text-sm font-semibold text-surface-600">
                  工作区标题
                  <input
                    value={draft.basic.workspace_title}
                    onChange={(event) => updateDraft({ basic: { ...draft.basic, workspace_title: event.target.value } })}
                    className="mt-2 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 text-sm outline-none focus:border-surface-500"
                  />
                </label>
                <label className="block text-sm font-semibold text-surface-600 md:col-span-2">
                  品牌副标题
                  <input
                    value={draft.basic.brand_tagline}
                    onChange={(event) => updateDraft({ basic: { ...draft.basic, brand_tagline: event.target.value } })}
                    className="mt-2 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 text-sm outline-none focus:border-surface-500"
                  />
                </label>
                <label className="block text-sm font-semibold text-surface-600">
                  默认主题
                  <select
                    value={draft.basic.default_theme}
                    onChange={(event) => updateDraft({ basic: { ...draft.basic, default_theme: event.target.value } })}
                    className="mt-2 w-full rounded-2xl border border-black/10 bg-white px-3 py-2.5 text-sm outline-none focus:border-surface-500"
                  >
                    {['default', 'business', 'tech', 'warm', 'minimal'].map((theme) => (
                      <option key={theme} value={theme}>{theme}</option>
                    ))}
                  </select>
                </label>
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

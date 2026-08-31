import { useCallback, useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { KeyRound, Copy, Trash2, Plus, Loader2, ExternalLink, Rocket, Sparkles, Check, Cpu, Network } from 'lucide-react'
import { llmApi } from '@/api'
import { useAuthStore } from '@/stores/auth-store'

const LOGO_URL = '/logo.png'

interface LiteKey {
  key_alias?: string
  key_name?: string
  models?: string[]
  spend?: number
  max_budget?: number | null
  expires?: string | null
  metadata?: Record<string, unknown>
}

export default function DashboardPage() {
  const navigate = useNavigate()
  const { user, logout } = useAuthStore()
  const [keys, setKeys] = useState<LiteKey[]>([])
  const [models, setModels] = useState<string[]>([])
  const [loading, setLoading] = useState(true)
  const [creating, setCreating] = useState(false)
  const [newName, setNewName] = useState('')
  const [newBudget, setNewBudget] = useState('')
  const [freshKey, setFreshKey] = useState<{ key: string; alias: string } | null>(null)
  const [copied, setCopied] = useState(false)
  const [error, setError] = useState('')

  const apiBase = `${window.location.origin}/v1`

  const refresh = useCallback(async () => {
    try {
      const [keysRes, modelsRes] = await Promise.allSettled([llmApi.listKeys(), llmApi.listModels()])
      if (keysRes.status === 'fulfilled') {
        setKeys(keysRes.value.data?.keys || [])
      } else {
        setError('API Key 列表加载失败：网关未就绪')
      }
      if (modelsRes.status === 'fulfilled') {
        const data = modelsRes.value.data?.data || []
        setModels(Array.isArray(data) ? data.map((m: any) => m.model_name || m.id).filter(Boolean) : [])
      }
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    refresh()
  }, [refresh])

  const handleCreate = async () => {
    if (!newName.trim()) return
    setCreating(true)
    setError('')
    try {
      const res = await llmApi.createKey({
        name: newName.trim(),
        budget: newBudget ? Number(newBudget) : null,
        duration: '90d',
      })
      const key = res.data?.key || res.data?.token || ''
      if (key) {
        setFreshKey({ key, alias: newName.trim() })
      }
      setNewName('')
      setNewBudget('')
      await refresh()
    } catch (err: any) {
      setError(err?.response?.data?.detail || '创建 Key 失败')
    } finally {
      setCreating(false)
    }
  }

  const handleRevoke = async (keyId: string) => {
    if (!confirm('确认吊销该 API Key？吊销后使用它的客户端将立即失效。')) return
    try {
      await llmApi.revokeKey(keyId)
      await refresh()
    } catch (err: any) {
      setError(err?.response?.data?.detail || '吊销失败')
    }
  }

  const handleCopy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch {
      // fallback
      const ta = document.createElement('textarea')
      ta.value = text
      document.body.appendChild(ta)
      ta.select()
      document.execCommand('copy')
      document.body.removeChild(ta)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    }
  }

  const handleLogout = () => {
    logout()
    navigate('/login')
  }

  return (
    <div className="mx-auto w-full max-w-5xl px-4 py-10 lg:px-6">
      {/* Hero */}
      <div className="flex flex-col gap-6 md:flex-row md:items-center md:justify-between">
        <div>
          <div className="flex items-center gap-3">
            <div className="flex h-12 w-12 items-center justify-center overflow-hidden rounded-2xl bg-white shadow-sm ring-1 ring-black/[0.06]">
              <img src={LOGO_URL} alt="logo" className="h-full w-full object-cover" />
            </div>
            <div>
              <h1 className="text-2xl font-black tracking-tight text-surface-950">算力工作台</h1>
              <p className="text-sm text-surface-500">
                {user?.username || ''} · 由 DGX Spark 提供本地大模型推理算力
              </p>
            </div>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={() => navigate('/office')}
            className="inline-flex items-center gap-2 rounded-full bg-surface-950 px-5 py-2.5 text-sm font-semibold text-white shadow-[0_10px_30px_rgba(24,24,27,0.18)] transition hover:bg-surface-800"
          >
            <Sparkles className="h-4 w-4" />
            进入智能助手
          </button>
          <button
            onClick={handleLogout}
            className="inline-flex items-center gap-1.5 rounded-full bg-white/55 px-4 py-2.5 text-sm font-medium text-surface-500 transition hover:bg-white/80 hover:text-surface-950"
          >
            退出登录
          </button>
        </div>
      </div>

      {error && (
        <div className="mt-6 rounded-2xl border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-800">{error}</div>
      )}

      {/* API Key 管理 */}
      <section className="mt-8 rounded-3xl border border-black/[0.06] bg-white/70 p-6 shadow-[0_12px_40px_rgba(24,24,27,0.05)] backdrop-blur">
        <div className="flex items-center gap-2">
          <KeyRound className="h-5 w-5 text-surface-700" />
          <h2 className="text-lg font-bold text-surface-950">API Key 管理</h2>
          <span className="rounded-full bg-emerald-100 px-2 py-0.5 text-[11px] font-medium text-emerald-700">LiteLLM 网关</span>
        </div>
        <p className="mt-1 text-sm text-surface-500">
          创建 API Key 后即可通过 OpenAI 兼容接口调用算力。Key 按月度预算可选限额，可随时吊销。
        </p>

        {/* 新建 Key */}
        <div className="mt-4 flex flex-wrap items-end gap-3">
          <label className="block text-sm">
            <span className="font-medium text-surface-600">Key 名称</span>
            <input
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              placeholder="例如：notebook-dev"
              className="mt-1.5 block w-56 rounded-xl border border-black/10 bg-white px-3 py-2 text-sm outline-none focus:border-surface-500"
            />
          </label>
          <label className="block text-sm">
            <span className="font-medium text-surface-600">月度预算（美元，可空）</span>
            <input
              value={newBudget}
              onChange={(e) => setNewBudget(e.target.value.replace(/[^\d.]/g, ''))}
              placeholder="例如：10"
              className="mt-1.5 block w-40 rounded-xl border border-black/10 bg-white px-3 py-2 text-sm outline-none focus:border-surface-500"
            />
          </label>
          <button
            onClick={handleCreate}
            disabled={creating || !newName.trim()}
            className="inline-flex h-[38px] items-center gap-1.5 rounded-xl bg-surface-950 px-4 text-sm font-semibold text-white transition hover:bg-surface-800 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {creating ? <Loader2 className="h-4 w-4 animate-spin" /> : <Plus className="h-4 w-4" />}
            创建 Key
          </button>
        </div>

        {/* 新建成功的一次性展示 */}
        {freshKey && (
          <div className="mt-4 rounded-2xl border border-emerald-200 bg-emerald-50 p-4">
            <div className="flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="text-sm font-semibold text-emerald-900">Key「{freshKey.alias}」创建成功（只显示一次，请立即保存）</div>
                <code className="mt-1 block break-all rounded-lg bg-white/70 px-3 py-2 font-mono text-xs text-emerald-800">{freshKey.key}</code>
              </div>
              <div className="flex shrink-0 gap-2">
                <button
                  onClick={() => handleCopy(freshKey.key)}
                  className="inline-flex items-center gap-1 rounded-full bg-emerald-600 px-3 py-1.5 text-xs font-semibold text-white hover:bg-emerald-700"
                >
                  {copied ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
                  {copied ? '已复制' : '复制'}
                </button>
                <button onClick={() => setFreshKey(null)} className="rounded-full px-2 py-1.5 text-xs font-medium text-emerald-700 hover:bg-emerald-100">
                  关闭
                </button>
              </div>
            </div>
          </div>
        )}

        {/* 已有 Key 列表 */}
        <div className="mt-4 overflow-hidden rounded-2xl border border-surface-200">
          {loading ? (
            <div className="flex items-center justify-center py-8 text-sm text-surface-400">
              <Loader2 className="mr-2 h-4 w-4 animate-spin" /> 加载中…
            </div>
          ) : keys.length === 0 ? (
            <div className="py-8 text-center text-sm text-surface-400">还没有 API Key，创建一个开始使用算力吧。</div>
          ) : (
            <table className="w-full border-collapse bg-white text-sm">
              <thead>
                <tr className="border-b border-surface-100 bg-surface-50 text-left text-xs text-surface-500">
                  <th className="px-4 py-2.5 font-medium">名称</th>
                  <th className="px-4 py-2.5 font-medium">可用模型</th>
                  <th className="px-4 py-2.5 font-medium">消耗 / 预算</th>
                  <th className="px-4 py-2.5 font-medium">过期</th>
                  <th className="px-4 py-2.5 text-right font-medium">操作</th>
                </tr>
              </thead>
              <tbody>
                {keys.map((k) => {
                  const alias = (k.metadata?.alias as string) || k.key_alias || k.key_name || '未命名'
                  const budget = typeof k.max_budget === 'number' ? `$${k.max_budget}` : '不限'
                  return (
                    <tr key={k.key_name || alias} className="border-b border-surface-100 last:border-b-0">
                      <td className="px-4 py-3 font-medium text-surface-800">{alias}</td>
                      <td className="px-4 py-3 text-xs text-surface-500">
                        {k.models && k.models.length > 0 ? k.models.slice(0, 3).join(', ') + (k.models.length > 3 ? '…' : '') : '全部'}
                      </td>
                      <td className="px-4 py-3 text-xs text-surface-600">
                        ${(k.spend || 0).toFixed(4)} / {budget}
                      </td>
                      <td className="px-4 py-3 text-xs text-surface-500">{k.expires ? String(k.expires).slice(0, 10) : '长期'}</td>
                      <td className="px-4 py-3 text-right">
                        <button
                          onClick={() => handleRevoke(k.key_name || '')}
                          className="inline-flex items-center gap-1 rounded-full border border-red-200 px-3 py-1 text-xs font-medium text-red-600 transition hover:bg-red-50"
                        >
                          <Trash2 className="h-3.5 w-3.5" /> 吊销
                        </button>
                      </td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
          )}
        </div>
      </section>

      {/* 帮助说明 */}
      <section className="mt-6 grid gap-6 lg:grid-cols-2">
        <div className="rounded-3xl border border-black/[0.06] bg-white/70 p-6 shadow-[0_12px_40px_rgba(24,24,27,0.05)] backdrop-blur lg:col-span-2">
          <div className="flex items-center gap-2">
            <Rocket className="h-5 w-5 text-surface-700" />
            <h2 className="text-lg font-bold text-surface-950">使用算力</h2>
          </div>
          <div className="mt-4 grid gap-4 md:grid-cols-2">
            <div className="rounded-2xl border border-surface-200 bg-white p-4">
              <div className="text-sm font-semibold text-surface-800">API 地址（OpenAI 兼容）</div>
              <div className="mt-2 flex items-center gap-2">
                <code className="flex-1 break-all rounded-lg bg-surface-50 px-3 py-2 font-mono text-xs text-surface-800">{apiBase}</code>
                <button onClick={() => handleCopy(apiBase)} className="rounded-full bg-surface-100 p-2 text-surface-500 hover:bg-surface-200" title="复制">
                  {copied ? <Check className="h-4 w-4 text-emerald-600" /> : <Copy className="h-4 w-4" />}
                </button>
              </div>
              <div className="mt-3 text-sm font-semibold text-surface-800">可用模型</div>
              <div className="mt-2 flex flex-wrap gap-1.5">
                {models.length === 0 ? (
                  <span className="text-xs text-surface-400">（网关未就绪或暂无模型）</span>
                ) : (
                  models.map((m) => (
                    <span key={m} className="rounded-full bg-indigo-50 px-2.5 py-1 text-xs font-medium text-indigo-700 ring-1 ring-indigo-100">
                      {m}
                    </span>
                  ))
                )}
              </div>
            </div>
            <div className="rounded-2xl border border-surface-200 bg-white p-4">
              <div className="text-sm font-semibold text-surface-800">快速开始（Python）</div>
              <pre className="mt-2 overflow-x-auto rounded-lg bg-surface-950 p-3 text-[11px] leading-5 text-emerald-300">{`from openai import OpenAI

client = OpenAI(
    base_url="${apiBase}",
    api_key="sk-你的APIKey",
)
resp = client.chat.completions.create(
    model="${models[0] || 'qwen3-30b-a3b'}",
    messages=[{"role": "user", "content": "你好"}],
)
print(resp.choices[0].message.content)`}</pre>
              <div className="mt-2 text-sm font-semibold text-surface-800">curl</div>
              <pre className="mt-1 overflow-x-auto rounded-lg bg-surface-950 p-3 text-[11px] leading-5 text-emerald-300">{`curl ${apiBase}/chat/completions \\
  -H "Authorization: Bearer sk-你的APIKey" \\
  -H "Content-Type: application/json" \\
  -d '{"model": "${models[0] || 'qwen3-30b-a3b'}", "messages": [{"role": "user", "content": "你好"}]}'`}</pre>
            </div>
          </div>
        </div>

        {/* 算力与架构说明 */}
        <div className="rounded-3xl border border-black/[0.06] bg-white/70 p-6 shadow-[0_12px_40px_rgba(24,24,27,0.05)] backdrop-blur">
          <div className="flex items-center gap-2">
            <Cpu className="h-5 w-5 text-surface-700" />
            <h2 className="text-lg font-bold text-surface-950">算力来源</h2>
          </div>
          <ul className="mt-3 space-y-2.5 text-sm leading-6 text-surface-600">
            <li className="flex gap-2"><span className="mt-2 h-1.5 w-1.5 shrink-0 rounded-full bg-emerald-500" />推理算力由本地 DGX Spark（<code className="rounded bg-surface-100 px-1">spark-7d76</code>，NVIDIA GB10 · 128GB 统一内存）提供</li>
            <li className="flex gap-2"><span className="mt-2 h-1.5 w-1.5 shrink-0 rounded-full bg-emerald-500" />云端门户与局域网算力机通过 frp 内网穿透打通，请求经 LiteLLM 网关统一鉴权、限流与计费</li>
            <li className="flex gap-2"><span className="mt-2 h-1.5 w-1.5 shrink-0 rounded-full bg-emerald-500" />Key 泄露可随时在「API Key 管理」吊销；预算超限自动停止</li>
          </ul>
        </div>

        {/* 智能助手入口 */}
        <div className="rounded-3xl border border-black/[0.06] bg-white/70 p-6 shadow-[0_12px_40px_rgba(24,24,27,0.05)] backdrop-blur">
          <div className="flex items-center gap-2">
            <Sparkles className="h-5 w-5 text-surface-700" />
            <h2 className="text-lg font-bold text-surface-950">办公助手</h2>
          </div>
          <p className="mt-2 text-sm leading-6 text-surface-600">
            基于 WaLiOffice 魔改的智能办公工作台：对话式生成 PPT、Word、Excel、流程图、图片与视频，支持飞书文档联动与 NAS 数据源。
          </p>
          <button
            onClick={() => navigate('/office')}
            className="mt-4 inline-flex items-center gap-1.5 rounded-full bg-surface-950 px-4 py-2 text-sm font-semibold text-white transition hover:bg-surface-800"
          >
            打开办公助手 <ExternalLink className="h-3.5 w-3.5" />
          </button>
        </div>
      </section>

      <footer className="mt-10 flex items-center justify-center gap-1.5 pb-4 text-xs text-surface-400">
        <Network className="h-3.5 w-3.5" />
        spark1.lab207.cn · 算力由 spark-7d76（DGX Spark）提供 · LiteLLM 统一网关
      </footer>
    </div>
  )
}

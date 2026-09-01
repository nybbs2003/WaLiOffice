import { useEffect, useRef, useState } from 'react'
import { ChevronDown, X, Loader2, RefreshCw } from 'lucide-react'

export interface ModelOptionItem {
  id: string
  /** 不可勾选（能力不符，仅列出名字） */
  disabled?: boolean
  /** 灰掉原因标签（如 不支持工具调用 / 非生图模型） */
  badge?: string
}

interface ModelComboboxProps {
  /** 已选中的模型列表 */
  models: string[]
  /** 可选的完整模型列表（拉取到的真实列表，与已选合并；支持 {id,disabled,badge} 结构） */
  options: (string | ModelOptionItem)[]
  /** 正在拉取中 */
  loading?: boolean
  /** 选择变化回调 */
  onChange: (models: string[]) => void
  /** 拉取模型回调 */
  onFetch?: () => void
}

/** 统一成条目结构 */
function toItem(o: string | ModelOptionItem): ModelOptionItem {
  return typeof o === 'string' ? { id: o } : o
}

/**
 * 模型选择 combobox：点击展开下拉，可输入过滤，选中后显示为可删除的标签。
 * 不具备能力的模型仅列出名字（灰色 + 原因标签），不可勾选。
 */
export function ModelCombobox({ models, options, loading, onChange, onFetch }: ModelComboboxProps) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const containerRef = useRef<HTMLDivElement>(null)

  // 点击外部关闭
  useEffect(() => {
    if (!open) return
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false)
        setQuery('')
      }
    }
    document.addEventListener('mousedown', handleClickOutside)
    return () => document.removeEventListener('mousedown', handleClickOutside)
  }, [open])

  // 可选项 = 已选 + options（去重），并按 query 过滤
  const allOptions: ModelOptionItem[] = (() => {
    const seen = new Map<string, ModelOptionItem>()
    for (const m of models) seen.set(m, { id: m })
    for (const o of options) {
      const item = toItem(o)
      if (!seen.has(item.id)) seen.set(item.id, item)
    }
    return Array.from(seen.values())
  })()
  const filtered = allOptions.filter((m) => m.id.toLowerCase().includes(query.trim().toLowerCase()))

  const toggleModel = (model: string) => {
    if (models.includes(model)) {
      onChange(models.filter((m) => m !== model))
    } else {
      onChange([...models, model])
    }
  }

  const removeModel = (model: string) => {
    onChange(models.filter((m) => m !== model))
  }

  return (
    <div ref={containerRef} className="relative">
      {/* 触发框：显示已选标签 + 下拉箭头 */}
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="flex min-h-[42px] w-full items-center gap-1.5 rounded-2xl border border-black/10 bg-white px-3 py-2 text-left text-sm outline-none transition focus:border-surface-500"
      >
        <div className="flex flex-1 flex-wrap items-center gap-1.5">
          {models.length === 0 ? (
            <span className="text-surface-400">点击选择模型…</span>
          ) : (
            models.map((m) => (
              <span
                key={m}
                className="inline-flex items-center gap-1 rounded-full bg-primary-50 px-2 py-0.5 text-[11px] font-medium text-primary-700"
              >
                {m}
                <span
                  role="button"
                  tabIndex={0}
                  onClick={(e) => { e.stopPropagation(); removeModel(m) }}
                  onKeyDown={(e) => { if (e.key === 'Enter') { e.stopPropagation(); removeModel(m) } }}
                  className="cursor-pointer rounded-full hover:bg-primary-100"
                >
                  <X className="h-3 w-3" />
                </span>
              </span>
            ))
          )}
        </div>
        <ChevronDown className={`h-4 w-4 shrink-0 text-surface-400 transition-transform ${open ? 'rotate-180' : ''}`} />
      </button>

      {/* 下拉面板 */}
      {open && (
        <div className="absolute left-0 right-0 top-full z-50 mt-1.5 max-h-64 overflow-hidden rounded-2xl border border-black/10 bg-white shadow-lg">
          {/* 搜索框 + 拉取按钮 */}
          <div className="flex items-center gap-2 border-b border-black/5 px-3 py-2">
            <input
              autoFocus
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="搜索或输入模型名…"
              className="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-surface-300"
              onKeyDown={(e) => {
                if (e.key === 'Enter' && query.trim() && !filtered.some((m) => m.id === query.trim())) {
                  // 允许手动输入新模型名并添加
                  toggleModel(query.trim())
                  setQuery('')
                }
              }}
            />
            {onFetch && (
              <button
                type="button"
                onClick={onFetch}
                disabled={loading}
                className="inline-flex shrink-0 items-center gap-1 rounded-full bg-primary-50 px-2.5 py-1 text-[11px] font-semibold text-primary-600 transition hover:bg-primary-100 disabled:opacity-50"
              >
                {loading ? <Loader2 className="h-3 w-3 animate-spin" /> : <RefreshCw className="h-3 w-3" />}
                拉取
              </button>
            )}
          </div>

          {/* 选项列表 */}
          <div className="max-h-48 overflow-y-auto p-1.5">
            {filtered.length === 0 ? (
              <div className="px-3 py-6 text-center text-xs text-surface-400">
                {query.trim() ? `按回车添加「${query.trim()}」` : '暂无模型，点「拉取」获取'}
              </div>
            ) : (
              filtered.map((item) => {
                const selected = models.includes(item.id)
                const locked = !!item.disabled
                return (
                  <button
                    key={item.id}
                    type="button"
                    onClick={() => { if (!locked) toggleModel(item.id) }}
                    disabled={locked}
                    className={`flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm transition ${
                      locked
                        ? 'cursor-not-allowed opacity-55'
                        : selected
                          ? 'bg-primary-50 text-primary-700'
                          : 'text-surface-700 hover:bg-surface-50'
                    }`}
                    title={locked ? (item.badge || '不可选') : item.id}
                  >
                    <span className={`flex h-4 w-4 shrink-0 items-center justify-center rounded border ${locked ? 'border-surface-200 bg-surface-100' : selected ? 'border-primary-500 bg-primary-500' : 'border-surface-300'}`}>
                      {selected && (
                        <svg className="h-3 w-3 text-white" fill="none" stroke="currentColor" viewBox="0 0 24 24" strokeWidth={3}>
                          <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
                        </svg>
                      )}
                    </span>
                    <span className="min-w-0 flex-1 truncate">{item.id}</span>
                    {item.badge && (
                      <span className="shrink-0 rounded-full bg-surface-100 px-1.5 py-0.5 text-[9px] font-medium text-surface-400">
                        {item.badge}
                      </span>
                    )}
                  </button>
                )
              })
            )}
          </div>
        </div>
      )}
    </div>
  )
}

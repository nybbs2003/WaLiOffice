import { useState, useRef, useEffect } from 'react'
import { createPortal } from 'react-dom'
import { ChevronDown, Settings2 } from 'lucide-react'
import { getAgentTool } from '@/config/agent-tools'
import type { ModelOptionSet, ToolKind, ToolConfigMap, ToolConfigOption } from '@/types'

interface ToolConfigDropdownProps {
  activeTool: ToolKind
  toolConfig: ToolConfigMap
  onToolConfigChange: (config: ToolConfigMap) => void
  disabled?: boolean
  /** 模型下拉的动态选项（来自用户多媒体配置），key=model 时使用 */
  modelOptions?: ModelOptionSet
}

/**
 * 工具配置下拉菜单，替代原来的主题选择器。
 * 根据当前 activeTool 显示对应的配置项。
 */
export function ToolConfigDropdown({
  activeTool,
  toolConfig,
  onToolConfigChange,
  disabled = false,
  modelOptions,
}: ToolConfigDropdownProps) {
  const [open, setOpen] = useState(false)
  const [pos, setPos] = useState<{ top: number; right: number } | null>(null)
  const buttonRef = useRef<HTMLButtonElement>(null)
  const panelRef = useRef<HTMLDivElement>(null)

  const tool = getAgentTool(activeTool)
  const options = tool.configOptions
  const hasConfig = options && options.length > 0

  // 点击外部关闭
  useEffect(() => {
    if (!open) return
    const handleClickOutside = (e: MouseEvent) => {
      const target = e.target as Node
      const inButton = buttonRef.current?.contains(target)
      const inPanel = panelRef.current?.contains(target)
      if (!inButton && !inPanel) {
        setOpen(false)
      }
    }
    document.addEventListener('mousedown', handleClickOutside)
    return () => document.removeEventListener('mousedown', handleClickOutside)
  }, [open])

  // Esc 关闭下拉
  useEffect(() => {
    if (!open) return
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    document.addEventListener('keydown', handleKey)
    return () => document.removeEventListener('keydown', handleKey)
  }, [open])

  // 切换工具时自动填充默认值（模型项默认值 = 设置里启用配置的默认模型）
  useEffect(() => {
    if (!options) return
    const defaults: ToolConfigMap = {}
    for (const opt of options) {
      if (!(opt.key in toolConfig)) {
        defaults[opt.key] = opt.key === 'model' && modelOptions?.defaultModel
          ? modelOptions.defaultModel
          : opt.defaultValue
      }
    }
    if (Object.keys(defaults).length > 0) {
      onToolConfigChange({ ...toolConfig, ...defaults })
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeTool])

  const handleChange = (key: string, value: string | boolean) => {
    onToolConfigChange({ ...toolConfig, [key]: value })
  }

  // 模型选项：优先动态（用户多媒体配置的启用配置），否则用工具内置预设
  const resolveOptions = (opt: ToolConfigOption): ToolConfigOption['options'] | undefined => {
    if (opt.key === 'model' && modelOptions && modelOptions.options.length > 0) {
      return modelOptions.options
    }
    return opt.options
  }

  // 有效默认值：模型项用设置里的默认模型
  const effectiveDefault = (opt: ToolConfigOption): string | boolean => {
    if (opt.key === 'model' && modelOptions?.defaultModel) {
      return modelOptions.defaultModel
    }
    return opt.defaultValue
  }

  const getActiveLabel = (opt: ToolConfigOption): string => {
    const val = toolConfig[opt.key] ?? effectiveDefault(opt)
    if (opt.type === 'toggle') return val ? '开' : '关'
    const match = resolveOptions(opt)?.find((o) => o.value === val)
    return match?.label ?? String(val)
  }

  const toggle = () => {
    if (!open && buttonRef.current) {
      const rect = buttonRef.current.getBoundingClientRect()
      // 面板向上弹出：底部对齐按钮顶部
      setPos({ top: rect.top, right: window.innerWidth - rect.right })
    }
    setOpen(!open)
  }

  // 检查是否有非默认配置
  const hasNonDefault = hasConfig
    ? options!.some((opt) => {
        const val = toolConfig[opt.key] ?? effectiveDefault(opt)
        return val !== effectiveDefault(opt)
      })
    : false

  // 按钮上只显示第一项配置摘要，保持简短
  const firstLabel = hasConfig ? getActiveLabel(options![0]) : '默认'

  // 没有配置项时，显示简化按钮
  if (!hasConfig) {
    return (
      <div className="inline-flex h-9 items-center gap-1.5 rounded-full border border-black/[0.06] bg-white px-2.5 text-[11px] font-medium text-surface-700 shadow-[0_1px_2px_rgba(15,23,42,0.04)]">
        <Settings2 className="h-3.5 w-3.5 text-surface-400" />
        <span className="text-surface-500">配置</span>
        <span className="font-semibold text-surface-900">默认</span>
      </div>
    )
  }

  return (
    <div>
      <button
        ref={buttonRef}
        type="button"
        disabled={disabled}
        onClick={toggle}
        className={`
          inline-flex h-9 items-center gap-1.5 rounded-full border px-2.5 text-[11px] font-medium
          transition-all disabled:opacity-50
          ${
            open
              ? 'border-indigo-200 bg-indigo-50 text-indigo-700 shadow-sm'
              : hasNonDefault
                ? 'border-amber-200 bg-amber-50 text-amber-700 shadow-sm hover:bg-amber-100'
                : 'border-black/[0.06] bg-white text-surface-700 shadow-[0_1px_2px_rgba(15,23,42,0.04)] hover:-translate-y-[0.5px] hover:shadow-[0_6px_14px_rgba(15,23,42,0.08)]'
          }
        `}
      >
        <Settings2 className={`h-3.5 w-3.5 ${open ? 'text-indigo-500' : hasNonDefault ? 'text-amber-500' : 'text-surface-400'}`} />
        <span className={open ? 'text-indigo-500' : hasNonDefault ? 'text-amber-500' : 'text-surface-500'}>配置</span>
        <span className={`font-semibold ${open ? 'text-indigo-700' : 'text-surface-900'}`}>{firstLabel}</span>
        {hasNonDefault && !open && (
          <span className="h-1.5 w-1.5 rounded-full bg-amber-500" />
        )}
        <ChevronDown className={`h-3 w-3 ${open ? 'text-indigo-400' : 'text-surface-400'} transition-transform ${open ? 'rotate-180' : ''}`} />
      </button>

      {open && pos && createPortal(
        <div
          ref={panelRef}
          className="fixed z-[100] w-72 rounded-2xl border border-gray-200 bg-white shadow-lg"
          style={{ top: pos.top - 8, right: pos.right, transform: 'translateY(-100%)' }}
        >
          {/* 标题栏 */}
          <div className="flex items-center justify-between px-4 py-2.5 border-b border-gray-100 bg-gray-50">
            <div className="flex items-center gap-2">
              <Settings2 className="h-4 w-4 text-indigo-500" />
              <span className="text-xs font-semibold text-surface-900">
                {tool.name}配置
              </span>
            </div>
            <button
              onClick={() => setOpen(false)}
              className="text-gray-400 hover:text-gray-600 transition-colors"
            >
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          </div>

          {/* 配置项列表 */}
          <div className="p-3 space-y-3">
            {options!.map((opt) => (
              <div key={opt.key}>
                <label className="block text-[11px] font-semibold text-surface-500 mb-1.5">
                  {opt.label}
                </label>

                {opt.type === 'select' && opt.key === 'model' && (() => {
                  // 模型项：下拉框（跟推理模型选择一致），默认值 = 设置里启用配置的默认模型
                  const opts = resolveOptions(opt)
                  if (!opts || opts.length === 0) return null
                  const currentValue = toolConfig[opt.key] ?? effectiveDefault(opt)
                  return (
                    <select
                      value={String(currentValue)}
                      onChange={(event) => handleChange(opt.key, event.target.value)}
                      className="w-full rounded-xl border border-gray-200 bg-white px-3 py-2 text-xs font-medium text-surface-800 outline-none focus:border-indigo-400 focus:ring-2 focus:ring-indigo-100"
                    >
                      {opts.map((option) => (
                        <option key={option.value} value={option.value} title={option.description}>
                          {option.label}
                        </option>
                      ))}
                    </select>
                  )
                })()}

                {opt.type === 'select' && opt.key !== 'model' && (() => {
                  const opts = resolveOptions(opt)
                  if (!opts) return null
                  return (
                    <div className="flex flex-wrap gap-1.5">
                      {opts.map((option) => {
                        const currentValue = toolConfig[opt.key] ?? opt.defaultValue
                        const isActive = currentValue === option.value
                        return (
                          <button
                            key={option.value}
                            onClick={() => handleChange(opt.key, option.value)}
                            className={`
                              flex items-center gap-1 px-2.5 py-1.5 rounded-lg text-[11px] font-medium transition-all
                              ${isActive
                                ? 'bg-indigo-500 text-white shadow-sm'
                                : 'bg-gray-50 text-surface-600 hover:bg-indigo-50 border border-gray-100'
                              }
                            `}
                            title={option.description}
                          >
                            {option.label}
                          </button>
                        )
                      })}
                    </div>
                  )
                })()}

                {opt.type === 'toggle' && (
                  <button
                    onClick={() => handleChange(opt.key, !(toolConfig[opt.key] ?? opt.defaultValue))}
                    className={`
                      relative inline-flex h-5 w-9 items-center rounded-full transition-colors
                      ${(toolConfig[opt.key] ?? opt.defaultValue) ? 'bg-indigo-500' : 'bg-gray-300'}
                    `}
                  >
                    <span
                      className={`
                        inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform
                        ${(toolConfig[opt.key] ?? opt.defaultValue) ? 'translate-x-4.5' : 'translate-x-0.5'}
                      `}
                    />
                  </button>
                )}
              </div>
            ))}
          </div>

          {/* 底部摘要 */}
          <div className="px-4 py-2 border-t border-gray-100 bg-gray-50">
            <div className="flex flex-wrap gap-1.5">
              {options!.map((opt) => (
                <span
                  key={opt.key}
                  className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] bg-gray-100 text-surface-500"
                >
                  <span className="font-medium">{opt.label}</span>
                  <span className="text-surface-700">{getActiveLabel(opt)}</span>
                </span>
              ))}
            </div>
          </div>
        </div>,
        document.body
      )}
    </div>
  )
}

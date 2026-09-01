import { useCallback, useEffect, useRef, useState } from 'react'
import { audioApi } from '@/api'
import { blobToBase64, startRecording, type RecordingHandles } from '@/lib/audioRecorder'

/**
 * 流式会议录音状态机：
 * - 边录边分块上传（每 ~8s 一块）
 * - 断网时块暂存 localStorage（moe_stream_*），联网后自动补传
 * - 轮询 worker 滑窗转写（后文更正上文）
 * - 转写更新后防抖调用增量纪要（后文更新前文）
 */

const LS_PREFIX = '***'

export interface StreamQueueItem {
  sid: string
  chunks: { seq: number; b64: string; final?: boolean }[]
  stopped: boolean
}

export interface MeetingRecorderState {
  phase: 'idle' | 'recording' | 'processing'
  seconds: number
  liveTranscript: string
  minutes: string
  pendingCount: number
  error: string | null
}

function lsAllStreams(): StreamQueueItem[] {
  const out: StreamQueueItem[] = []
  for (let i = 0; i < localStorage.length; i++) {
    const k = localStorage.key(i)
    if (k && k.startsWith(LS_PREFIX)) {
      try {
        out.push(JSON.parse(localStorage.getItem(k) || '{}'))
      } catch {
        /* 忽略损坏项 */
      }
    }
  }
  return out
}

function lsSaveStream(item: StreamQueueItem) {
  localStorage.setItem(LS_PREFIX + item.sid, JSON.stringify(item))
}

function lsRemoveStream(sid: string) {
  localStorage.removeItem(LS_PREFIX + sid)
}

export function useMeetingRecorder(onComplete: (transcript: string) => void) {
  const [state, setState] = useState<MeetingRecorderState>({
    phase: 'idle',
    seconds: 0,
    liveTranscript: '',
    minutes: '',
    pendingCount: lsAllStreams().reduce((n, s) => n + s.chunks.length, 0),
    error: null,
  })

  const sidRef = useRef<string | null>(null)
  const recorderRef = useRef<RecordingHandles | null>(null)
  const analyserRef = useRef<AnalyserNode | null>(null)
  const timerRef = useRef<number | null>(null)
  const pollRef = useRef<number | null>(null)
  const uploadBusyRef = useRef(false)
  const minutesTimerRef = useRef<number | null>(null)
  const minutesBusyRef = useRef(false)
  const lastVersionRef = useRef(0)
  const stableCounterRef = useRef(0)
  const stoppedRef = useRef(false)
  const completedRef = useRef(false)
  const stateRef = useRef(state)
  stateRef.current = state

  const uploadPending = useCallback(async (): Promise<boolean> => {
    if (uploadBusyRef.current) return true
    uploadBusyRef.current = true
    try {
      let progressed = false
      for (const item of lsAllStreams()) {
        let ok = true
        const remaining = [...item.chunks].sort((a, b) => a.seq - b.seq)
        for (const c of remaining) {
          try {
            await audioApi.streamChunk(item.sid, { b64: c.b64, seq: c.seq, final: !!c.final })
            item.chunks = item.chunks.filter((x) => x.seq !== c.seq)
            lsSaveStream(item)
            progressed = true
          } catch {
            ok = false
            break // 网络不行，保留剩余
          }
        }
        if (item.chunks.length === 0) {
          lsRemoveStream(item.sid)
        } else if (!ok) {
          break
        }
      }
      setState((s) => ({ ...s, pendingCount: lsAllStreams().reduce((n, x) => n + x.chunks.length, 0) }))
      return progressed
    } finally {
      uploadBusyRef.current = false
    }
  }, [])

  const pollTranscript = useCallback(async () => {
    const sid = sidRef.current
    if (!sid) return
    // 优先补传积压块（网络恢复场景）
    await uploadPending()
    try {
      const res = await audioApi.streamTranscript(sid)
      const d = res.data
      const changed = d.version !== lastVersionRef.current || d.text !== stateRef.current.liveTranscript
      lastVersionRef.current = d.version
      if (changed) {
        stableCounterRef.current = 0
      }
      if (changed) {
        setState((s) => ({ ...s, liveTranscript: d.text }))
        // 防抖触发增量纪要（转写安静 12s 后更新）
        if (d.text.trim()) {
          if (minutesTimerRef.current) window.clearTimeout(minutesTimerRef.current)
          minutesTimerRef.current = window.setTimeout(async () => {
            if (minutesBusyRef.current) return
            minutesBusyRef.current = true
            try {
              const mr = await audioApi.streamMinutes(sid, {
                transcript: stateRef.current.liveTranscript,
                prev: stateRef.current.minutes,
              })
              setState((s) => ({ ...s, minutes: mr.data.markdown }))
            } catch {
              /* 纪要失败不阻断录音 */
            } finally {
              minutesBusyRef.current = false
            }
          }, 12000)
        }
      }
      // 录音结束后的收尾
      if (stoppedRef.current && !completedRef.current) {
        const queueEmpty = lsAllStreams().filter((x) => x.sid === sid).length === 0
        if (queueEmpty && !d.transcribing && d.text.trim()) {
          // 连续两轮（跨轮次）版本不变才算收敛
          if (stableCounterRef.current < 1) {
            stableCounterRef.current += 1
            return
          }
          completedRef.current = true
          const finalRes = await audioApi.streamFinish(sid).catch(() => null)
          const finalText = finalRes?.data?.text || d.text
          const finalMinutes = stateRef.current.minutes
          const combined = finalText + (finalMinutes ? '\n\n【实时纪要】\n' + finalMinutes : '')
          setState((s) => ({ ...s, liveTranscript: finalText, phase: 'idle', minutes: finalMinutes }))
          sidRef.current = null
          if (pollRef.current) window.clearInterval(pollRef.current)
          onComplete(combined)
          return
        }
      }
    } catch {
      /* 网络抖动：下轮继续 */
    }
  }, [onComplete, uploadPending])

  const start = useCallback(async () => {
    if (stateRef.current.phase !== 'idle') return
    stoppedRef.current = false
    completedRef.current = false
    lastVersionRef.current = 0
    stableCounterRef.current = 0
    const sid = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`
    sidRef.current = sid
    setState({ phase: 'recording', seconds: 0, liveTranscript: '', minutes: '', pendingCount: stateRef.current.pendingCount, error: null })
    try {
      const handles = await startRecording({
        chunkSeconds: 8,
        onChunk: async ({ blob, seq }) => {
          try {
            const b64 = await blobToBase64(blob)
            // 先进 localStorage（网不好也不丢），再尝试上传
            const key = LS_PREFIX + sid
            const item: StreamQueueItem = JSON.parse(localStorage.getItem(key) || '{"sid":"","chunks":[],"stopped":false}')
            item.sid = sid
            item.chunks.push({ seq, b64 })
            lsSaveStream(item)
            setState((s) => ({ ...s, pendingCount: lsAllStreams().reduce((n, x) => n + x.chunks.length, 0) }))
            await uploadPending()
          } catch {
            /* localStorage 满等场景：块已在本地队列外，忽略并继续录音 */
          }
        },
      })
      recorderRef.current = handles
      analyserRef.current = handles.analyser
      timerRef.current = window.setInterval(() => {
        setState((s) => (s.phase === 'recording' ? { ...s, seconds: s.seconds + 1 } : s))
      }, 1000)
      pollRef.current = window.setInterval(() => {
        pollTranscript()
      }, 2500)
    } catch (err) {
      setState((s) => ({ ...s, phase: 'idle', error: `无法开始录音：${err instanceof Error ? err.message : String(err)}（需要麦克风权限）` }))
    }
  }, [pollTranscript, uploadPending])

  const stop = useCallback(async () => {
    if (stateRef.current.phase !== 'recording' || !recorderRef.current) return
    stoppedRef.current = true
    if (timerRef.current) window.clearInterval(timerRef.current)
    const handles = recorderRef.current
    recorderRef.current = null
    await handles.stop() // 触发 final 块 flush
    // 标记队列结束 + 最后一块 final=true（若还有积压，补传完会触发收尾）
    const sid = sidRef.current
    if (sid) {
      const key = LS_PREFIX + sid
      const item: StreamQueueItem = JSON.parse(localStorage.getItem(key) || '{"sid":"","chunks":[],"stopped":false}')
      item.sid = sid
      item.stopped = true
      const last = item.chunks[item.chunks.length - 1]
      if (last) last.final = true
      lsSaveStream(item)
    }
    setState((s) => ({ ...s, phase: 'processing' }))
    // 收尾由 pollTranscript 检测收敛后触发
    await pollTranscript()
  }, [pollTranscript])

  const retrySync = useCallback(async () => {
    await uploadPending()
    await pollTranscript()
  }, [pollTranscript, uploadPending])

  // 页面加载：自动补传历史积压
  useEffect(() => {
    const t = window.setTimeout(() => {
      uploadPending()
    }, 1500)
    return () => {
      window.clearTimeout(t)
      if (timerRef.current) window.clearInterval(timerRef.current)
      if (pollRef.current) window.clearInterval(pollRef.current)
      if (minutesTimerRef.current) window.clearTimeout(minutesTimerRef.current)
    }
  }, [uploadPending])

  return { state, start, stop, retrySync, sid: sidRef.current, analyserRef }
}

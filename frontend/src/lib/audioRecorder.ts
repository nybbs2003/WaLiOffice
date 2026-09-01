/**
 * 本地录音（Web Audio API 波形 + MediaRecorder 压缩编码）：
 * - 浏览器内置编码：优先 Opus（audio/webm），Safari 用 AAC（audio/mp4），兜底默认格式
 * - 录音体积约为 WAV 的 1/8~1/10，网络传输与 NAS 存储均为有损压缩音频
 * - 波形可视化仍走 Web Audio API（AnalyserNode）
 */

export interface RecordingHandles {
  /** 停止录音（返回的 blob 为最后一块；完整数据经 onChunk 分块流出） */
  stop(): Promise<{ blob: Blob; durationSec: number }>
  /** 波形数据（供 UI 绘制） */
  analyser: AnalyserNode | null
  /** 采样上下文（录制中保持存活） */
  context: AudioContext
}

export interface RecordingOptions {
  /** 流式分块回调：每约 chunkSeconds 秒触发一次（边录边传） */
  onChunk?: (chunk: { blob: Blob; seq: number; offsetSec: number; durationSec: number }) => void
  chunkSeconds?: number
}

function pickMimeType(): string | undefined {
  if (typeof MediaRecorder === 'undefined') return undefined
  const candidates = [
    'audio/webm;codecs=opus',
    'audio/webm',
    'audio/mp4',
    'audio/ogg;codecs=opus',
    'audio/mpeg',
  ]
  for (const c of candidates) {
    try {
      if (MediaRecorder.isTypeSupported(c)) return c
    } catch {
      /* 继续尝试 */
    }
  }
  return undefined
}

export async function startRecording(options: RecordingOptions = {}): Promise<RecordingHandles> {
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: { echoCancellation: true, noiseSuppression: true, channelCount: 1 },
  })
  const context = new AudioContext()
  const source = context.createMediaStreamSource(stream)

  const analyser = context.createAnalyser()
  analyser.fftSize = 256
  source.connect(analyser)

  const mimeType = pickMimeType()
  const recorder = mimeType ? new MediaRecorder(stream, { mimeType }) : new MediaRecorder(stream)
  const chunkSeconds = (options.chunkSeconds ?? 8) * 1000

  let seq = 0
  let startedAt = Date.now()
  let finished = false

  recorder.ondataavailable = (event: BlobEvent) => {
    if (!event.data || event.data.size === 0) return
    seq += 1
    options.onChunk?.({
      blob: event.data,
      seq,
      offsetSec: (Date.now() - startedAt) / 1000,
      durationSec: chunkSeconds / 1000,
    })
  }

  return {
    analyser,
    context,
    async stop() {
      if (finished) return { blob: new Blob(), durationSec: 0 }
      finished = true
      const durationSec = (Date.now() - startedAt) / 1000
      const finalBlob = await new Promise<Blob>((resolve) => {
        const onStop = () => {
          resolve(new Blob())
        }
        recorder.addEventListener('stop', onStop, { once: true })
        try {
          recorder.stop()
        } catch {
          resolve(new Blob())
        }
      })
      // 等待 ondataavailable 把最后一块交出（stop 触发的 dataavailable 是异步的）
      await new Promise((r) => setTimeout(r, 80))
      try {
        stream.getTracks().forEach((t) => t.stop())
        source.disconnect()
      } catch {
        /* 忽略清理异常 */
      }
      await context.close().catch(() => undefined)
      return { blob: finalBlob, durationSec }
    },
  }
}

export function blobToBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(String(reader.result).split(',')[1] || '')
    reader.onerror = () => reject(reader.error)
    reader.readAsDataURL(blob)
  })
}

/* ---------------- localStorage 暂存队列 ---------------- */

const LS_KEY = '***'

export interface PendingRecording {
  name: string
  b64: string
  duration: number
  ts: number
}

export function lsListRecordings(): PendingRecording[] {
  try {
    const raw = localStorage.getItem(LS_KEY)
    if (!raw) return []
    const arr = JSON.parse(raw)
    return Array.isArray(arr) ? arr : []
  } catch {
    return []
  }
}

/** 网不好时只存 localStorage；上传成功由调用方调用 lsRemoveRecording 删除 */
export function lsSaveRecording(name: string, b64: string, duration: number): void {
  const list = lsListRecordings()
  list.push({ name, b64, duration, ts: Date.now() })
  try {
    localStorage.setItem(LS_KEY, JSON.stringify(list))
  } catch {
    const trimmed = list.slice(1)
    try {
      localStorage.setItem(LS_KEY, JSON.stringify(trimmed))
    } catch {
      throw new Error('本地暂存空间不足：录音过长或浏览器存储已满')
    }
  }
}

export function lsRemoveRecording(name: string): void {
  const list = lsListRecordings().filter((r) => r.name !== name)
  localStorage.setItem(LS_KEY, JSON.stringify(list))
}

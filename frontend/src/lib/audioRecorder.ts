/**
 * 本地录音（Web Audio API）：getUserMedia → AudioContext → PCM 采集 → WAV 编码。
 * 兼容所有现代浏览器（Chrome/Edge/Safari/Firefox，含 HTTPS 或 localhost 环境）。
 */

export interface RecordingHandles {
  /** 停止录音并得到 WAV Blob 与时长（秒） */
  stop(): Promise<{ blob: Blob; durationSec: number }>
  /** 波形数据（供 UI 绘制，调用 getByteTimeDomainData） */
  analyser: AnalyserNode | null
  /** 采样上下文（录制中保持存活） */
  context: AudioContext
}

const TARGET_RATE = 16000

export async function startRecording(): Promise<RecordingHandles> {
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: { echoCancellation: true, noiseSuppression: true, channelCount: 1 },
  })
  const context = new AudioContext()
  const source = context.createMediaStreamSource(stream)

  const analyser = context.createAnalyser()
  analyser.fftSize = 256
  source.connect(analyser)

  // ScriptProcessorNode：兼容性最好（现代浏览器全部支持）
  const processor = context.createScriptProcessor(4096, 1, 1)
  // 静音增益节点，防止监听回路啸叫
  const mute = context.createGain()
  mute.gain.value = 0
  source.connect(processor)
  processor.connect(mute)
  mute.connect(context.destination)

  const chunks: Float32Array[] = []
  let sourceRate = context.sampleRate

  processor.onaudioprocess = (event: AudioProcessingEvent) => {
    const data = event.inputBuffer.getChannelData(0)
    chunks.push(new Float32Array(data))
  }

  return {
    analyser,
    context,
    async stop() {
      const durationSec = chunks.reduce((n, c) => n + c.length, 0) / sourceRate
      try {
        processor.disconnect()
        mute.disconnect()
        source.disconnect()
        stream.getTracks().forEach((t) => t.stop())
      } catch {
        /* 忽略清理异常 */
      }
      const merged = mergeChunks(chunks)
      const resampled = resampleLinear(merged, sourceRate, TARGET_RATE)
      const blob = encodeWav(resampled, TARGET_RATE)
      await context.close().catch(() => undefined)
      return { blob, durationSec }
    },
  }
}

function mergeChunks(chunks: Float32Array[]): Float32Array {
  const total = chunks.reduce((n, c) => n + c.length, 0)
  const out = new Float32Array(total)
  let offset = 0
  for (const c of chunks) {
    out.set(c, offset)
    offset += c.length
  }
  return out
}

/** 线性插值重采样 */
function resampleLinear(input: Float32Array, fromRate: number, toRate: number): Float32Array {
  if (fromRate === toRate) return input
  const ratio = fromRate / toRate
  const length = Math.floor(input.length / ratio)
  const out = new Float32Array(length)
  for (let i = 0; i < length; i++) {
    const pos = i * ratio
    const idx = Math.floor(pos)
    const frac = pos - idx
    const a = input[idx] ?? 0
    const b = input[idx + 1] ?? a
    out[i] = a + (b - a) * frac
  }
  return out
}

/** Float32 PCM → 16-bit 单声道 WAV */
function encodeWav(samples: Float32Array, sampleRate: number): Blob {
  const buffer = new ArrayBuffer(44 + samples.length * 2)
  const view = new DataView(buffer)
  const writeString = (offset: number, s: string) => {
    for (let i = 0; i < s.length; i++) view.setUint8(offset + i, s.charCodeAt(i))
  }
  writeString(0, 'RIFF')
  view.setUint32(4, 36 + samples.length * 2, true)
  writeString(8, 'WAVE')
  writeString(12, 'fmt ')
  view.setUint32(16, 16, true)
  view.setUint16(20, 1, true) // PCM
  view.setUint16(22, 1, true) // mono
  view.setUint32(24, sampleRate, true)
  view.setUint32(28, sampleRate * 2, true)
  view.setUint16(32, 2, true)
  view.setUint16(34, 16, true)
  writeString(36, 'data')
  view.setUint32(40, samples.length * 2, true)
  let offset = 44
  for (let i = 0; i < samples.length; i++, offset += 2) {
    const s = Math.max(-1, Math.min(1, samples[i]))
    view.setInt16(offset, s < 0 ? s * 0x8000 : s * 0x7fff, true)
  }
  return new Blob([buffer], { type: 'audio/wav' })
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

const LS_KEY = 'moe_audio_pending'

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
    // 超出容量：丢弃最早的暂存再试一次
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

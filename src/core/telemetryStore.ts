import { formatBytes, formatEta, formatSpeed } from './segmentation';

const MAGIC_ARG3 = 0x41524733;
const encoder = typeof TextEncoder !== 'undefined' ? new TextEncoder() : null;

/**
 * Matches Rust `dual_hash_id(id)` byte-for-byte without allocating objects.
 */
export function dualHashId(id: string): [number, number] {
  let h1 = 2166136261 >>> 0;
  let h2 = 5381 >>> 0;
  if (encoder) {
    const bytes = encoder.encode(id);
    for (let i = 0; i < bytes.length; i++) {
      const b = bytes[i];
      h1 ^= b;
      h1 = Math.imul(h1, 16777619) >>> 0;
      h2 = (Math.imul(h2, 33) + b) >>> 0;
    }
  } else {
    for (let i = 0; i < id.length; i++) {
      const b = id.charCodeAt(i) & 0xff;
      h1 ^= b;
      h1 = Math.imul(h1, 16777619) >>> 0;
      h2 = (Math.imul(h2, 33) + b) >>> 0;
    }
  }
  return [h1, h2];
}

/**
 * Parses a binary telemetry frame returned by Rust `poll_telemetry_binary`
 * using a single DataView without JSON parsing or intermediate heap allocations.
 */
export function parseBinaryTelemetryFrame(
  buffer: ArrayBuffer,
  onRecord: (h1: number, h2: number, downloaded: number, total: number, speedBps: number) => void
): number {
  if (!buffer || buffer.byteLength < 16) return 0;
  const view = new DataView(buffer);
  if (view.getUint32(0, true) !== MAGIC_ARG3) return 0;
  const count = view.getUint32(4, true);
  const globalSpeed = view.getFloat64(8, true);
  let offset = 16;
  for (let i = 0; i < count && offset + 32 <= buffer.byteLength; i++) {
    const h1 = view.getUint32(offset, true);
    const h2 = view.getUint32(offset + 4, true);
    const downloaded = view.getFloat64(offset + 8, true);
    const total = view.getFloat64(offset + 16, true);
    const speedBps = view.getFloat64(offset + 24, true);
    onRecord(h1, h2, downloaded, total, speedBps);
    offset += 32;
  }
  return globalSpeed;
}

export interface TaskDomBinding {
  progressBar: HTMLElement | null;
  bytesText: HTMLElement | null;
  speedText: HTMLElement | null;
  etaText: HTMLElement | null;
  compactRateText: HTMLElement | null;
}

class ZeroGcTelemetryStore {
  private bindings = new Map<string, TaskDomBinding>();
  private nativeToTask = new Map<string, { taskId: string; h1: number; h2: number }>();
  private taskToNative = new Map<string, string>();
  private rafId: number | null = null;
  private lastPollAt = 0;
  private pollingInFlight = false;

  public bindNativeRun(taskId: string, nativeId: string | null) {
    const prev = this.taskToNative.get(taskId);
    if (prev) {
      this.nativeToTask.delete(prev);
      this.taskToNative.delete(taskId);
    }
    if (nativeId) {
      const [h1, h2] = dualHashId(nativeId);
      this.taskToNative.set(taskId, nativeId);
      this.nativeToTask.set(nativeId, { taskId, h1, h2 });
      this.ensureLoop();
    } else if (this.nativeToTask.size === 0) {
      this.stopLoop();
    }
  }

  public registerTaskDom(taskId: string, binding: TaskDomBinding) {
    this.bindings.set(taskId, binding);
  }

  public unregisterTaskDom(taskId: string) {
    this.bindings.delete(taskId);
  }

  public applyTaskMetrics(taskId: string, downloaded: number, total: number, speedBps: number) {
    const dom = this.bindings.get(taskId);
    if (!dom) return;

    const known = total > 0;
    const ratio = known ? Math.min(1, Math.max(0, downloaded / total)) : 0;
    const pct = known ? Math.min(100, Math.floor(ratio * 100)) : 0;
    const eta = known && speedBps > 0 ? Math.ceil(Math.max(0, total - downloaded) / speedBps) : 0;

    if (dom.progressBar) {
      dom.progressBar.style.transform = `scaleX(${ratio})`;
    }
    if (dom.bytesText) {
      dom.bytesText.textContent = known
        ? `${formatBytes(downloaded)} / ${formatBytes(total)}`
        : formatBytes(downloaded);
    }
    if (dom.speedText) {
      dom.speedText.textContent = speedBps > 0 ? formatSpeed(speedBps) : known && downloaded > 0 ? `${pct}%` : '—';
    }
    if (dom.etaText) {
      dom.etaText.textContent = eta > 0 ? formatEta(eta) : '';
    }
    if (dom.compactRateText) {
      dom.compactRateText.textContent =
        speedBps > 0 ? `${formatSpeed(speedBps)}${eta > 0 ? ` · ${formatEta(eta)}` : ''}` : '';
    }
  }

  private ensureLoop() {
    if (this.rafId !== null || typeof window === 'undefined' || typeof requestAnimationFrame === 'undefined') return;
    if (!('__TAURI_INTERNALS__' in window)) return;

    const step = (now: number) => {
      if (this.nativeToTask.size === 0) {
        this.rafId = null;
        return;
      }
      if (typeof document !== 'undefined' && document.visibilityState === 'hidden') {
        this.rafId = requestAnimationFrame(step);
        return;
      }
      if (now - this.lastPollAt >= 33 && !this.pollingInFlight) {
        this.lastPollAt = now;
        this.pollingInFlight = true;
        void import('@tauri-apps/api/core')
          .then(({ invoke }) => invoke<ArrayBuffer>('poll_telemetry_binary'))
          .then((buffer) => {
            if (buffer instanceof ArrayBuffer) {
              parseBinaryTelemetryFrame(buffer, (h1, h2, downloaded, total, speedBps) => {
                for (const entry of this.nativeToTask.values()) {
                  if (entry.h1 === h1 && entry.h2 === h2) {
                    this.applyTaskMetrics(entry.taskId, downloaded, total, speedBps);
                    break;
                  }
                }
              });
            }
          })
          .catch(() => {})
          .finally(() => {
            this.pollingInFlight = false;
          });
      }
      this.rafId = requestAnimationFrame(step);
    };

    this.rafId = requestAnimationFrame(step);
  }

  private stopLoop() {
    if (this.rafId !== null && typeof cancelAnimationFrame !== 'undefined') {
      cancelAnimationFrame(this.rafId);
      this.rafId = null;
    }
  }
}

export const telemetryStore = new ZeroGcTelemetryStore();

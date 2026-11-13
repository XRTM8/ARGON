import { nativeRuntime } from './flowClient';

/**
 * Reads text from the system clipboard across Android, desktop, and web environments.
 *
 * 1. Under Tauri runtime (Android APK or desktop app), it invokes the native backend
 *    command `flow_read_clipboard`. This accesses the OS clipboard directly (e.g. Android's
 *    ClipboardManager), completely bypassing WebView security restrictions that block
 *    navigator.clipboard.readText() and prompt mobile users with irrelevant desktop "Ctrl+V" messages.
 *
 * 2. In browser environments, it falls back to `navigator.clipboard.readText()`.
 */
export async function readClipboardText(): Promise<string> {
  if (nativeRuntime()) {
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const text = await invoke<string>('flow_read_clipboard');
      if (typeof text === 'string') {
        return text;
      }
    } catch {
      // Fallback to web clipboard if native invoke fails
    }
  }

  if (typeof navigator !== 'undefined' && navigator.clipboard && typeof navigator.clipboard.readText === 'function') {
    return await navigator.clipboard.readText();
  }

  throw new Error('Clipboard unavailable');
}

/**
 * Writes text to the system clipboard across Android, desktop, and web environments.
 * Uses native Tauri commands when available, falling back to navigator.clipboard.writeText.
 */
export async function writeClipboardText(text: string): Promise<void> {
  if (nativeRuntime()) {
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('flow_write_clipboard', { text });
      return;
    } catch {
      // Fallback to web clipboard if native command is unavailable or fails
    }
  }

  if (typeof navigator !== 'undefined' && navigator.clipboard && typeof navigator.clipboard.writeText === 'function') {
    await navigator.clipboard.writeText(text);
    return;
  }

  throw new Error('Clipboard unavailable');
}

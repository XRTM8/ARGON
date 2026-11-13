export interface SafeAreaInsets {
  top: number;
  bottom: number;
  left: number;
  right: number;
  ime: number;
}

declare global {
  interface Window {
    __ARGON_INSETS__?: SafeAreaInsets;
    ArgonNative?: {
      getInsetsJson?: () => string;
      consumeSharedUrl?: () => string;
      setSystemBarTheme?: (light: boolean) => void;
    };
  }
}

export function isAndroidRuntime(): boolean {
  return typeof navigator !== 'undefined' && /android/i.test(navigator.userAgent);
}

export function applyInsetsToDocument(insets: Partial<SafeAreaInsets>): void {
  if (typeof document === 'undefined') return;
  const style = document.documentElement?.style;
  if (!style) return;
  if (typeof insets.top === 'number' && Number.isFinite(insets.top)) {
    style.setProperty('--native-sat', `${Math.max(0, insets.top)}px`);
  }
  if (typeof insets.bottom === 'number' && Number.isFinite(insets.bottom)) {
    style.setProperty('--native-sab', `${Math.max(0, insets.bottom)}px`);
  }
  if (typeof insets.left === 'number' && Number.isFinite(insets.left)) {
    style.setProperty('--native-sal', `${Math.max(0, insets.left)}px`);
  }
  if (typeof insets.right === 'number' && Number.isFinite(insets.right)) {
    style.setProperty('--native-sar', `${Math.max(0, insets.right)}px`);
  }
  if (typeof insets.ime === 'number' && Number.isFinite(insets.ime)) {
    style.setProperty('--native-ime', `${Math.max(0, insets.ime)}px`);
  }
}

export function initSafeAreaInsets(): () => void {
  if (typeof window === 'undefined') return () => {};

  if (isAndroidRuntime()) {
    document.documentElement.dataset.platform = 'android';
  }

  // Apply synchronously from native bridge if available before first React paint
  try {
    const raw = window.ArgonNative?.getInsetsJson?.();
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<SafeAreaInsets>;
      applyInsetsToDocument(parsed);
    } else if (window.__ARGON_INSETS__) {
      applyInsetsToDocument(window.__ARGON_INSETS__);
    } else if (isAndroidRuntime()) {
      // Conservative safety floor until WindowInsets listener fires (42px accommodates camera punch-holes)
      applyInsetsToDocument({ top: 42, bottom: 18, left: 0, right: 0, ime: 0 });
    }
  } catch {
    if (isAndroidRuntime()) {
      applyInsetsToDocument({ top: 42, bottom: 18, left: 0, right: 0, ime: 0 });
    }
  }

  const onInsets = (event: Event) => {
    const detail = (event as CustomEvent<SafeAreaInsets>).detail;
    if (detail) applyInsetsToDocument(detail);
  };
  window.addEventListener('argon-insets', onInsets);
  return () => window.removeEventListener('argon-insets', onInsets);
}

export function syncSystemBarTheme(theme: 'dark' | 'light'): void {
  if (typeof document !== 'undefined') {
    document.documentElement.dataset.theme = theme;
  }
  try {
    window.ArgonNative?.setSystemBarTheme?.(theme === 'light');
  } catch {
    // Ignore on non-Android platforms
  }
}

export function consumeInitialAndroidSharedUrl(): string {
  try {
    return window.ArgonNative?.consumeSharedUrl?.()?.trim() || '';
  } catch {
    return '';
  }
}

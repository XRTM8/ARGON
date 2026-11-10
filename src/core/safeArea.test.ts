import { describe, expect, it, vi, afterEach } from 'vitest';
import { applyInsetsToDocument, initSafeAreaInsets } from './safeArea';

describe('safeArea runtime insets', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('writes native pixel insets to root CSS variables', () => {
    const vars = new Map<string, string>();
    vi.stubGlobal('document', {
      documentElement: {
        dataset: {},
        style: { setProperty: (k: string, v: string) => vars.set(k, v) }
      }
    });
    applyInsetsToDocument({ top: 38, bottom: 24, left: 0, right: 0, ime: 280 });
    expect(vars.get('--native-sat')).toBe('38px');
    expect(vars.get('--native-sab')).toBe('24px');
    expect(vars.get('--native-ime')).toBe('280px');
  });

  it('reads synchronous Android bridge insets before first paint and updates on event', () => {
    const vars = new Map<string, string>();
    const listeners = new Map<string, (e: any) => void>();
    vi.stubGlobal('navigator', { userAgent: 'Mozilla/5.0 (Linux; Android 15)' });
    vi.stubGlobal('document', {
      documentElement: {
        dataset: {},
        style: { setProperty: (k: string, v: string) => vars.set(k, v) }
      }
    });
    vi.stubGlobal('window', {
      ArgonNative: { getInsetsJson: () => '{"top":42,"bottom":18,"left":0,"right":0,"ime":0}' },
      addEventListener: (name: string, fn: (e: any) => void) => listeners.set(name, fn),
      removeEventListener: (name: string) => listeners.delete(name)
    });

    const cleanup = initSafeAreaInsets();
    expect(vars.get('--native-sat')).toBe('42px');
    expect(vars.get('--native-sab')).toBe('18px');

    listeners.get('argon-insets')?.({ detail: { top: 44, bottom: 20, left: 12, right: 12, ime: 0 } });
    expect(vars.get('--native-sat')).toBe('44px');
    expect(vars.get('--native-sal')).toBe('12px');
    cleanup();
  });
});

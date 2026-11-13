import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { readClipboardText, writeClipboardText } from './clipboard';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

describe('clipboard helper', () => {
  beforeEach(() => {
    invoke.mockReset();
    vi.stubGlobal('window', {});
    vi.stubGlobal('navigator', {});
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('reads text from navigator.clipboard when not in native runtime', async () => {
    const mockReadText = vi.fn().mockResolvedValue('https://example.com/test.mp4');
    vi.stubGlobal('navigator', {
      clipboard: {
        readText: mockReadText
      }
    });

    const result = await readClipboardText();
    expect(result).toBe('https://example.com/test.mp4');
    expect(mockReadText).toHaveBeenCalledTimes(1);
    expect(invoke).not.toHaveBeenCalled();
  });

  it('writes text to navigator.clipboard when not in native runtime', async () => {
    const mockWriteText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal('navigator', {
      clipboard: {
        writeText: mockWriteText
      }
    });

    await writeClipboardText('https://example.com/copied.zip');
    expect(mockWriteText).toHaveBeenCalledWith('https://example.com/copied.zip');
    expect(invoke).not.toHaveBeenCalled();
  });

  it('invokes native flow_read_clipboard and flow_write_clipboard under native runtime', async () => {
    vi.stubGlobal('window', { __TAURI_INTERNALS__: {} });
    invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === 'flow_read_clipboard') return 'https://instagram.com/reel/123';
      if (command === 'flow_write_clipboard') {
        expect(args).toEqual({ text: 'https://youtube.com/watch?v=abc' });
        return undefined;
      }
      throw new Error(`Unknown command ${command}`);
    });

    const text = await readClipboardText();
    expect(text).toBe('https://instagram.com/reel/123');
    expect(invoke).toHaveBeenCalledWith('flow_read_clipboard');

    await writeClipboardText('https://youtube.com/watch?v=abc');
    expect(invoke).toHaveBeenCalledWith('flow_write_clipboard', { text: 'https://youtube.com/watch?v=abc' });
  });

  it('falls back to navigator.clipboard if native invoke fails', async () => {
    vi.stubGlobal('window', { __TAURI_INTERNALS__: {} });
    invoke.mockRejectedValue(new Error('native bridge unavailable'));
    const mockReadText = vi.fn().mockResolvedValue('https://fallback.com/video');
    vi.stubGlobal('navigator', {
      clipboard: {
        readText: mockReadText
      }
    });

    const result = await readClipboardText();
    expect(result).toBe('https://fallback.com/video');
  });

  it('throws when clipboard API is unavailable and not in native runtime', async () => {
    vi.stubGlobal('navigator', {});

    await expect(readClipboardText()).rejects.toThrow('Clipboard unavailable');
    await expect(writeClipboardText('foo')).rejects.toThrow('Clipboard unavailable');
  });
});

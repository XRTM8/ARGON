import { describe, expect, it } from 'vitest';
import { dualHashId, parseBinaryTelemetryFrame } from './telemetryStore';

describe('zero-GC binary telemetry protocol', () => {
  it('produces stable 64-bit dual hashes for task identifiers', () => {
    const [h1a, h2a] = dualHashId('task_1');
    const [h1b, h2b] = dualHashId('task_1');
    const [h1c, h2c] = dualHashId('task_2');
    expect(h1a).toBe(h1b);
    expect(h2a).toBe(h2b);
    expect([h1a, h2a]).not.toEqual([h1c, h2c]);
  });

  it('decodes packed ARG3 binary telemetry frame without JSON parsing', () => {
    const buffer = new ArrayBuffer(16 + 32);
    const view = new DataView(buffer);
    const [h1, h2] = dualHashId('task_alpha');
    view.setUint32(0, 0x41524733, true);
    view.setUint32(4, 1, true);
    view.setFloat64(8, 12_500_000, true);
    view.setUint32(16, h1, true);
    view.setUint32(20, h2, true);
    view.setFloat64(24, 5_000_000, true);
    view.setFloat64(32, 10_000_000, true);
    view.setFloat64(40, 12_500_000, true);

    const rows: Array<[number, number, number, number, number]> = [];
    const globalSpeed = parseBinaryTelemetryFrame(buffer, (a, b, dl, tot, spd) => {
      rows.push([a, b, dl, tot, spd]);
    });
    expect(globalSpeed).toBe(12_500_000);
    expect(rows).toEqual([[h1, h2, 5_000_000, 10_000_000, 12_500_000]]);
  });

  it('ignores malformed magic headers safely', () => {
    const buffer = new ArrayBuffer(16);
    const view = new DataView(buffer);
    view.setUint32(0, 0xdeadbeef, true);
    const seen: number[] = [];
    expect(parseBinaryTelemetryFrame(buffer, (a) => seen.push(a))).toBe(0);
    expect(seen).toHaveLength(0);
  });
});

import { describe, expect, it } from 'vitest';
import jsQR from 'jsqr';
import makeQrCode from 'qrcode-generator';
import { createPairingInvite, parsePairingInvite } from './pairingInvite';
import type { FlowSnapshot } from '../types/flow';

describe('local pairing invitations', () => {
  const now = 1_800_000_000_000;
  const snapshot = {
    enabled: true,
    addresses: ['127.0.0.1:54127', '192.168.1.4:54127'],
    id: 'a'.repeat(64)
  } as FlowSnapshot;

  it('decodes a short-lived local invitation from a QR code', () => {
    const invite = createPairingInvite(snapshot, now)!;
    const code = makeQrCode(0, 'M');
    code.addData(invite);
    code.make();
    const moduleSize = 4;
    const border = 4;
    const size = (code.getModuleCount() + border * 2) * moduleSize;
    const pixels = new Uint8ClampedArray(size * size * 4).fill(255);
    for (let row = 0; row < code.getModuleCount(); row++) {
      for (let col = 0; col < code.getModuleCount(); col++) {
        if (!code.isDark(row, col)) continue;
        for (let y = 0; y < moduleSize; y++) {
          for (let x = 0; x < moduleSize; x++) {
            const index = (((row + border) * moduleSize + y) * size + (col + border) * moduleSize + x) * 4;
            pixels.fill(0, index, index + 3);
          }
        }
      }
    }
    const decoded = jsQR(pixels, size, size);
    expect(parsePairingInvite(decoded!.data, now)).toEqual({ address: '192.168.1.4:54127', peerId: snapshot.id, expiresAt: now + 120_000 });
  });

  it('rejects nonlocal, expired and malformed invitations', () => {
    const invite = createPairingInvite(snapshot, now)!;
    expect(parsePairingInvite(invite, now + 120_000)).toBeNull();
    expect(parsePairingInvite(invite.replace('192.168.1.4', '8.8.8.8'), now)).toBeNull();
    expect(parsePairingInvite(invite.replace(snapshot.id, 'unknown'), now)).toBeNull();
    expect(createPairingInvite({ ...snapshot, addresses: ['127.0.0.1:54127'] }, now)).toBeNull();
  });
});

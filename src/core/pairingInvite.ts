import type { FlowSnapshot } from '../types/flow';

const invitePrefix = 'argon-flow/2:pair:';
const inviteLifetime = 120_000;

export function createPairingInvite(snapshot: FlowSnapshot, now = Date.now()): string | null {
  const address = snapshot.addresses.find((value) => !value.startsWith('127.') && isLocalAddress(value));
  if (!snapshot.enabled || !address) return null;
  return invitePrefix + JSON.stringify({ address, peerId: snapshot.id, expiresAt: now + inviteLifetime });
}

function isLocalAddress(value: string): boolean {
  const match = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3}):(\d{1,5})$/.exec(value);
  if (!match) return false;
  const [first, second, third, fourth, port] = match.slice(1).map(Number);
  return [first, second, third, fourth].every((part) => part <= 255)
    && port > 0 && port <= 65_535
    && (first === 10 || first === 127 || (first === 172 && second >= 16 && second <= 31)
      || (first === 192 && second === 168) || (first === 169 && second === 254));
}

export function parsePairingInvite(value: string, now = Date.now()): {
  address: string;
  peerId: string;
  expiresAt: number;
} | null {
  if (!value.startsWith(invitePrefix) || value.length > 256) return null;
  try {
    const data = JSON.parse(value.slice(invitePrefix.length));
    if (typeof data?.address !== 'string' || !isLocalAddress(data.address)
      || typeof data.peerId !== 'string' || !/^[0-9a-f]{64}$/.test(data.peerId)
      || !Number.isSafeInteger(data.expiresAt) || data.expiresAt <= now || data.expiresAt > now + 180_000) return null;
    return { address: data.address, peerId: data.peerId, expiresAt: data.expiresAt };
  } catch {
    return null;
  }
}

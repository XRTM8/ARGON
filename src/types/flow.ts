export interface FlowPeer { id:string; name:string; address:string; trusted:boolean; online:boolean; lastSeen:number }
export interface FlowTransfer {
  id:string; peerId:string; peerName:string; name:string; direction:'send'|'receive'; kind:'file'|'text';
  status:'queued'|'hashing'|'waiting'|'sending'|'receiving'|'verifying'|'completed'|'error'|'cancelled';
  bytes:number; total:number; speed:number; createdAt:number; path:string|null; error:string|null; sha256:string|null;
}
export interface FlowPairing { id:string; peerId:string; peerName:string; code:string; expiresAt:number }
export interface FlowIncoming { id:string; peerId:string; peerName:string; name:string; size:number; kind:'file'|'text'; expiresAt:number }
export interface FlowClipboard { id:string; peerName:string; text:string; receivedAt:number }
export interface FlowSelectedFile { id:string; name:string; size:number }
export interface FlowSnapshot {
  enabled:boolean; id:string; name:string; port:number; addresses:string[]; receiveDirectory:string;
  peers:FlowPeer[]; transfers:FlowTransfer[]; pairings:FlowPairing[]; incoming:FlowIncoming[];
  clipboard:FlowClipboard[]; lastError:string|null; protocol:string;
}

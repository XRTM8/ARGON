export type PlatformType = 'youtube' | 'tiktok' | 'instagram' | 'twitter' | 'soundcloud' | 'direct' | 'other';

export interface MediaFormatOption {
  id: string; label: string; labelAr: string; ext: string; resolution?: string;
  qualityBadge: string; isAudioOnly: boolean; estimatedSizeMultiplier?: number;
}
export interface MediaPreview {
  originalUrl: string; platform: PlatformType; title: string; author?: string;
  thumbnail?: string; durationSeconds?: number; availableFormats: MediaFormatOption[]; resolvedStreamUrl?: string;
}

export function detectPlatform(rawUrl: string): PlatformType {
  try {
    const url = new URL(rawUrl.trim());
    const host = url.hostname.toLowerCase();
    const matches = (domain: string) => host === domain || host.endsWith(`.${domain}`);
    if (matches('youtube.com') || matches('youtu.be')) return 'youtube';
    if (matches('tiktok.com')) return 'tiktok';
    if (matches('instagram.com')) return 'instagram';
    if (matches('twitter.com') || matches('x.com')) return 'twitter';
    if (matches('soundcloud.com')) return 'soundcloud';
    return /\.(mp4|mkv|webm|avi|mov|flv|ts|mp3|m4a|aac|flac|wav|ogg|opus)$/i.test(url.pathname) ? 'direct' : 'other';
  } catch { return 'other'; }
}

export function cleanMediaTitle(rawTitle: string): string {
  return rawTitle.replace(/[<>:"/\\|?*\x00-\x1f]/g, '_').replace(/\s+/g, ' ').trim().slice(0, 160) || 'media';
}

export async function resolveMediaDownloadUrl(url: string, option?: Pick<MediaFormatOption, 'isAudioOnly'>): Promise<{
  downloadUrl: string; suggestedFilename?: string; headers?: Record<string, string>;
}> {
  const cleanUrl = url.trim();
  if (['direct', 'other'].includes(detectPlatform(cleanUrl))) return { downloadUrl: cleanUrl };
  if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) {
    throw new Error('استخراج روابط المنصات يتطلب تطبيق ARGON لسطح المكتب.');
  }
  const { invoke } = await import('@tauri-apps/api/core');
  // Native extraction avoids CSP/CORS and preserves the required CDN headers.
  // Extraction failures must never fall back to downloading the webpage.
  return invoke('resolve_media', { url: cleanUrl, audioOnly: option?.isAudioOnly ?? false });
}

export async function fetchMediaMetadata(url: string): Promise<MediaPreview> {
  const result = await resolveMediaDownloadUrl(url);
  const parsed = new URL(result.downloadUrl);
  const title = result.suggestedFilename || decodeURIComponent(parsed.pathname.split('/').pop() || 'media');
  const ext = title.split('.').pop()?.toLowerCase() || 'bin';
  return {
    originalUrl: url, platform: detectPlatform(url), title, author: parsed.hostname,
    resolvedStreamUrl: result.downloadUrl,
    availableFormats: [{ id: 'original', label: `Original (${ext})`, labelAr: `الملف الأصلي (${ext})`, ext,
      qualityBadge: ext.toUpperCase(), isAudioOnly: /^(mp3|m4a|aac|flac|wav|ogg|opus)$/.test(ext) }],
  };
}

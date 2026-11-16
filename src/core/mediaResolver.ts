import type { MediaMetadataInfo, PlatformType } from '../types/downloader';

export type { PlatformType };

export interface MediaFormatOption {
  id: string;
  label: string;
  labelAr: string;
  ext: string;
  resolution?: string;
  qualityBadge: string;
  isAudioOnly: boolean;
  estimatedSizeMultiplier?: number;
}

export interface MediaPreview {
  originalUrl: string;
  platform: PlatformType;
  title: string;
  author?: string;
  thumbnail?: string;
  durationSeconds?: number;
  availableFormats: MediaFormatOption[];
  resolvedStreamUrl?: string;
}

export function detectPlatform(rawUrl: string): PlatformType {
  try {
    const url = new URL(rawUrl.trim());
    const host = url.hostname.toLowerCase();
    const matches = (domain: string) => host === domain || host.endsWith(`.${domain}`);
    if (matches('youtube.com') || matches('youtu.be')) return 'youtube';
    if (matches('tiktok.com')) return 'tiktok';
    if (matches('instagram.com') || matches('instagr.am') || matches('ig.me')) return 'instagram';
    if (matches('twitter.com') || matches('x.com') || matches('t.co')) return 'twitter';
    if (matches('facebook.com') || matches('fb.watch') || matches('fb.com')) return 'facebook';
    if (matches('reddit.com') || matches('redd.it') || matches('v.redd.it')) return 'reddit';
    if (matches('threads.net') || matches('threads.com')) return 'threads';
    if (matches('vimeo.com')) return 'vimeo';
    if (matches('soundcloud.com')) return 'soundcloud';
    if (matches('pinterest.com') || matches('pin.it') || host.includes('pinterest.')) return 'pinterest';
    if (matches('snapchat.com')) return 'snapchat';
    if (matches('linkedin.com')) return 'linkedin';
    if (matches('twitch.tv')) return 'twitch';
    if (matches('kick.com')) return 'kick';
    if (matches('dailymotion.com') || matches('dai.ly')) return 'dailymotion';
    if (matches('bilibili.com') || matches('b23.tv')) return 'bilibili';
    if (matches('kuaishou.com') || matches('kwai.com')) return 'kwai';
    if (matches('douyin.com')) return 'douyin';
    if (matches('bsky.app')) return 'bluesky';
    if (matches('tumblr.com')) return 'tumblr';
    if (matches('streamable.com')) return 'streamable';
    if (matches('loom.com')) return 'loom';
    if (matches('vk.com') || matches('vkvideo.ru')) return 'vk';
    if (matches('rumble.com')) return 'rumble';
    if (matches('odysee.com')) return 'odysee';
    if (matches('bandcamp.com')) return 'bandcamp';
    return /\.(mp4|mkv|webm|avi|mov|flv|ts|mp3|m4a|aac|flac|wav|ogg|opus)$/i.test(url.pathname) ? 'direct' : 'other';
  } catch { return 'other'; }
}

export function cleanMediaTitle(rawTitle: string): string {
  return rawTitle.replace(/[<>:"/\\|?*\x00-\x1f]/g, '_').replace(/\s+/g, ' ').trim().slice(0, 160) || 'media';
}

export function isExplicitBinaryFile(rawUrl: string): boolean {
  try {
    const url = new URL(rawUrl.trim());
    return /\.(zip|rar|7z|tar|gz|bz2|xz|zst|iso|img|bin|exe|msi|apk|dmg|deb|rpm|pdf|docx?|xlsx?|pptx?|csv|epub|torrent)$/i.test(url.pathname);
  } catch {
    return false;
  }
}

export async function listMediaFormats(url: string): Promise<MediaMetadataInfo> {
  if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) {
    throw new Error('استخراج روابط المنصات يتطلب تشغيل تطبيق ARGON.');
  }
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<MediaMetadataInfo>('list_media_formats', { url: url.trim() });
}

export async function resolveMediaDownloadUrl(
  url: string,
  option?: { isAudioOnly?: boolean; formatId?: string }
): Promise<{
  downloadUrl: string;
  suggestedFilename?: string;
  headers?: Record<string, string>;
}> {
  const cleanUrl = url.trim();
  const platform = detectPlatform(cleanUrl);
  if (platform === 'direct' || isExplicitBinaryFile(cleanUrl)) return { downloadUrl: cleanUrl };
  if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) {
    throw new Error('استخراج روابط المنصات يتطلب تشغيل تطبيق ARGON.');
  }
  const { invoke } = await import('@tauri-apps/api/core');
  try {
    return await invoke('resolve_media', {
      url: cleanUrl,
      audioOnly: option?.isAudioOnly ?? false,
      ...(option?.formatId ? { formatId: option.formatId } : {})
    });
  } catch (err) {
    if (platform === 'other') return { downloadUrl: cleanUrl };
    throw err;
  }
}

export async function fetchMediaMetadata(url: string): Promise<MediaPreview> {
  const result = await resolveMediaDownloadUrl(url);
  const parsed = new URL(result.downloadUrl);
  const title = result.suggestedFilename || decodeURIComponent(parsed.pathname.split('/').pop() || 'media');
  const ext = title.split('.').pop()?.toLowerCase() || 'bin';
  return {
    originalUrl: url,
    platform: detectPlatform(url),
    title,
    author: parsed.hostname,
    resolvedStreamUrl: result.downloadUrl,
    availableFormats: [
      {
        id: 'original',
        label: `Original (${ext})`,
        labelAr: `الملف الأصلي (${ext})`,
        ext,
        qualityBadge: ext.toUpperCase(),
        isAudioOnly: /^(mp3|m4a|aac|flac|wav|ogg|opus)$/.test(ext)
      }
    ]
  };
}

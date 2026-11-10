/** Parse without rewriting signed URLs or silently accepting invalid lines. */
export function parseDownloadInput(text: string) {
  const urls: string[] = [];
  const invalid: string[] = [];
  let duplicates = 0;
  const seen = new Set<string>();
  for (const raw of text.split(/\r?\n/)) {
    const value = raw.trim();
    if (!value) continue;
    try {
      const url = new URL(value);
      if (!['http:', 'https:'].includes(url.protocol) || !url.hostname) throw new Error();
      if (seen.has(value)) { duplicates++; continue; }
      seen.add(value);
      urls.push(value);
    } catch { invalid.push(value); }
  }
  return { urls, invalid, duplicates };
}

/**
 * Expands IDM-style numeric or alphabetic wildcard patterns:
 * e.g. https://example.com/archive_[01..10].zip or https://example.com/part_[a..e].bin
 */
export function expandWildcardPattern(pattern: string, maxUrls = 100): { urls: string[]; error?: string } {
  const trimmed = pattern.trim();
  const numMatch = trimmed.match(/\[(\d+)\.\.(\d+)\]/);
  if (numMatch) {
    const startStr = numMatch[1];
    const endStr = numMatch[2];
    const start = parseInt(startStr, 10);
    const end = parseInt(endStr, 10);
    if (Number.isNaN(start) || Number.isNaN(end) || end < start) {
      return { urls: [], error: 'invalid_range' };
    }
    const count = end - start + 1;
    if (count > maxUrls) {
      return { urls: [], error: 'range_too_large' };
    }
    const padWidth = startStr.length;
    const urls: string[] = [];
    for (let i = start; i <= end; i++) {
      const token = String(i).padStart(padWidth, '0');
      const candidate = trimmed.replace(numMatch[0], token);
      try {
        const parsed = new URL(candidate);
        if (!['http:', 'https:'].includes(parsed.protocol)) return { urls: [], error: 'invalid_url' };
        urls.push(candidate);
      } catch {
        return { urls: [], error: 'invalid_url' };
      }
    }
    return { urls };
  }

  const alphaMatch = trimmed.match(/\[([a-zA-Z])\.\.([a-zA-Z])\]/);
  if (alphaMatch) {
    const startCode = alphaMatch[1].charCodeAt(0);
    const endCode = alphaMatch[2].charCodeAt(0);
    const sameCase = (startCode >= 97 && endCode >= 97) || (startCode <= 90 && endCode <= 90);
    if (!sameCase || endCode < startCode) {
      return { urls: [], error: 'invalid_range' };
    }
    const urls: string[] = [];
    for (let code = startCode; code <= endCode; code++) {
      const candidate = trimmed.replace(alphaMatch[0], String.fromCharCode(code));
      try {
        const parsed = new URL(candidate);
        if (!['http:', 'https:'].includes(parsed.protocol)) return { urls: [], error: 'invalid_url' };
        urls.push(candidate);
      } catch {
        return { urls: [], error: 'invalid_url' };
      }
    }
    return { urls };
  }

  return { urls: [], error: 'no_wildcard' };
}

export function extractSharedUrl(raw: string): string {
  const match = raw.match(/https?:\/\/[^\s<>"{}|\\^`\[\]]+/i);
  if (!match) return raw.trim();
  return match[0].replace(/[.,!?;:)'"]+$/, '');
}

export function downloadHost(url: string) {
  try { return new URL(url).hostname.replace(/^www\./, ''); }
  catch { return ''; }
}

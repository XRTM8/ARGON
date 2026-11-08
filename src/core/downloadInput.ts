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

export function downloadHost(url: string) {
  try { return new URL(url).hostname.replace(/^www\./, ''); }
  catch { return ''; }
}

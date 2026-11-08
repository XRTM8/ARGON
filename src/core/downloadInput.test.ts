import { describe, expect, it } from 'vitest';
import { parseDownloadInput } from './downloadInput';

describe('batch download input', () => {
  it('preserves signed query strings and removes exact duplicates', () => {
    const url = 'https://cdn.example.com/file.zip?sig=a%2Bb&token=X%2FY';
    expect(parseDownloadInput(` ${url}\r\n${url}\nhttps://example.com/ملف.pdf`)).toEqual({ urls: [url, 'https://example.com/ملف.pdf'], invalid: [], duplicates: 1 });
  });
  it('reports invalid lines without dropping them or permitting local protocols', () => {
    const input = parseDownloadInput('file:///C:/secret\nhttps://\nmagnet:?xt=abc\nhttps://example.com/ok\nnot a link');
    expect(input.urls).toEqual(['https://example.com/ok']);
    expect(input.invalid).toHaveLength(4);
  });
});

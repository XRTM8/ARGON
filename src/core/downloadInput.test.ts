import { describe, expect, it } from 'vitest';
import { expandWildcardPattern, parseDownloadInput } from './downloadInput';

describe('batch download input', () => {
  it('preserves signed query strings and removes exact duplicates', () => {
    const url = 'https://cdn.example.com/file.zip?sig=a%2Bb&token=X%2FY';
    expect(parseDownloadInput(` ${url}\r\n${url}\nhttps://example.com/ملف.pdf`)).toEqual({
      urls: [url, 'https://example.com/ملف.pdf'],
      invalid: [],
      duplicates: 1
    });
  });

  it('reports invalid lines without dropping them or permitting local protocols', () => {
    const input = parseDownloadInput('file:///C:/secret\nhttps://\nmagnet:?xt=abc\nhttps://example.com/ok\nnot a link');
    expect(input.urls).toEqual(['https://example.com/ok']);
    expect(input.invalid).toHaveLength(4);
  });

  it('expands numeric and alphabetic wildcard ranges cleanly up to 100 links', () => {
    expect(expandWildcardPattern('https://cdn.example.com/part_[01..03].zip')).toEqual({
      urls: [
        'https://cdn.example.com/part_01.zip',
        'https://cdn.example.com/part_02.zip',
        'https://cdn.example.com/part_03.zip'
      ]
    });
    expect(expandWildcardPattern('https://cdn.example.com/vol_[a..c].bin')).toEqual({
      urls: [
        'https://cdn.example.com/vol_a.bin',
        'https://cdn.example.com/vol_b.bin',
        'https://cdn.example.com/vol_c.bin'
      ]
    });
    expect(expandWildcardPattern('https://cdn.example.com/part_[001..200].zip').error).toBe('range_too_large');
  });
});

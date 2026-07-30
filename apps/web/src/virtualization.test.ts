import { describe, expect, test } from 'bun:test'

import { virtualRange } from './virtualization'

describe('document list virtualization', () => {
  test('renders only a bounded window for a large corpus', () => {
    const range = virtualRange(100_000, 20_000, 320, 32, 5)
    expect(range).toEqual({
      start: 620,
      end: 640,
      offsetTop: 19_840,
      totalHeight: 3_200_000,
    })
  })

  test('clamps empty and end-of-list ranges', () => {
    expect(virtualRange(0, 0, 300, 32)).toEqual({
      start: 0,
      end: 0,
      offsetTop: 0,
      totalHeight: 0,
    })
    expect(virtualRange(10, 9_999, 300, 32, 2)).toEqual({
      start: 7,
      end: 10,
      offsetTop: 224,
      totalHeight: 320,
    })
  })
})

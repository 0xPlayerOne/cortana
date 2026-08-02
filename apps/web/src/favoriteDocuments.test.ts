import { afterEach, expect, test } from 'bun:test'

import { isFavoriteDocument, toggleFavoriteDocument } from './favoriteDocuments'

const STORAGE_KEY = 'cortana.favorite-documents.v1'

afterEach(() => {
  window.localStorage.removeItem(STORAGE_KEY)
})

test('favorites persist only opaque document identifiers and toggle deterministically', () => {
  expect(isFavoriteDocument('document-1')).toBe(false)
  expect(toggleFavoriteDocument('document-1')).toBe(true)
  expect(isFavoriteDocument('document-1')).toBe(true)
  expect(JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? 'null')).toEqual(['document-1'])

  expect(toggleFavoriteDocument('document-1')).toBe(false)
  expect(isFavoriteDocument('document-1')).toBe(false)
})

test('malformed local favorite state is ignored', () => {
  window.localStorage.setItem(STORAGE_KEY, '{not-json')
  expect(isFavoriteDocument('document-1')).toBe(false)
  expect(toggleFavoriteDocument('document-2')).toBe(true)
  expect(JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? 'null')).toEqual(['document-2'])
})

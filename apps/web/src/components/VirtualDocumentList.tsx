import { FileText } from 'lucide-react'
import { type KeyboardEvent, useEffect, useMemo, useRef, useState } from 'react'

import type { BrainDocumentSummary } from '../types'
import { virtualRange } from '../virtualization'
import { useM7SurfacePrimitives } from './m7/M7SurfacePrimitives'

const ROW_HEIGHT = 32

export function VirtualDocumentList({
  documents,
  selectedDocument,
  loading,
  hasMore,
  onSelect,
  onLoadMore,
  renderer = 'legacy',
}: {
  documents: BrainDocumentSummary[]
  selectedDocument: string
  loading: boolean
  hasMore: boolean
  onSelect: (id: string) => void
  onLoadMore: () => void
  renderer?: 'legacy' | 'shadcn'
}) {
  const ShadcnButton = useM7SurfacePrimitives()?.Button
  const viewportRef = useRef<HTMLDivElement>(null)
  const loadRequested = useRef(false)
  const [scrollTop, setScrollTop] = useState(0)
  const [viewportHeight, setViewportHeight] = useState(240)
  const selectedIndex = Math.max(
    0,
    documents.findIndex((document) => document.id === selectedDocument)
  )
  const [activeIndex, setActiveIndex] = useState(selectedIndex)

  useEffect(() => {
    setActiveIndex(Math.min(selectedIndex, Math.max(0, documents.length - 1)))
  }, [documents.length, selectedIndex])

  useEffect(() => {
    const viewport = viewportRef.current
    if (!viewport) return
    const observer = new ResizeObserver(([entry]) => setViewportHeight(entry.contentRect.height))
    observer.observe(viewport)
    return () => observer.disconnect()
  }, [])

  useEffect(() => {
    if (!loading) loadRequested.current = false
  }, [documents.length, loading])

  const range = useMemo(
    () => virtualRange(documents.length, scrollTop, viewportHeight, ROW_HEIGHT),
    [documents.length, scrollTop, viewportHeight]
  )

  function focusIndex(index: number) {
    if (!documents.length) return
    const next = Math.max(0, Math.min(documents.length - 1, index))
    setActiveIndex(next)
    const viewport = viewportRef.current
    if (!viewport) return
    const top = next * ROW_HEIGHT
    if (top < viewport.scrollTop) viewport.scrollTop = top
    else if (top + ROW_HEIGHT > viewport.scrollTop + viewport.clientHeight) {
      viewport.scrollTop = top + ROW_HEIGHT - viewport.clientHeight
    }
  }

  function handleKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (!documents.length) return
    if (event.key === 'ArrowDown') focusIndex(activeIndex + 1)
    else if (event.key === 'ArrowUp') focusIndex(activeIndex - 1)
    else if (event.key === 'Home') focusIndex(0)
    else if (event.key === 'End') focusIndex(documents.length - 1)
    else if (event.key === 'Enter') onSelect(documents[activeIndex].id)
    else return
    event.preventDefault()
  }

  return (
    <div
      ref={viewportRef}
      className="virtual-document-list"
      role="listbox"
      aria-label="Documents"
      aria-busy={loading}
      aria-activedescendant={
        documents[activeIndex] ? `document-option-${documents[activeIndex].id}` : undefined
      }
      tabIndex={0}
      onKeyDown={handleKeyDown}
      onScroll={(event) => {
        const viewport = event.currentTarget
        setScrollTop(viewport.scrollTop)
        if (
          hasMore &&
          !loading &&
          !loadRequested.current &&
          viewport.scrollTop + viewport.clientHeight >= viewport.scrollHeight - ROW_HEIGHT * 4
        ) {
          loadRequested.current = true
          onLoadMore()
        }
      }}
    >
      <div className="virtual-document-space" style={{ height: range.totalHeight }}>
        <div style={{ transform: `translateY(${range.offsetTop}px)` }}>
          {documents.slice(range.start, range.end).map((document, offset) => {
            const index = range.start + offset
            const content = (
              <>
                <FileText size={14} />
                <span>{document.title}</span>
                <small>{document.source}</small>
              </>
            )
            const sharedProps = {
              id: `document-option-${document.id}`,
              role: 'option',
              tabIndex: -1,
              'aria-selected': selectedDocument === document.id,
              className: [
                'document-node',
                selectedDocument === document.id ? 'selected-document' : '',
                activeIndex === index ? 'keyboard-active' : '',
              ]
                .filter(Boolean)
                .join(' '),
              style: { height: ROW_HEIGHT },
              onMouseEnter: () => setActiveIndex(index),
              onFocus: () => setActiveIndex(index),
              onClick: () => onSelect(document.id),
              title: `${document.title} · ${document.source}`,
            } as const
            return renderer === 'shadcn' && ShadcnButton ? (
              <ShadcnButton
                key={document.id}
                {...sharedProps}
                variant="ghost"
                size="sm"
                data-m7-document-row=""
              >
                {content}
              </ShadcnButton>
            ) : (
              <button key={document.id} {...sharedProps} type="button">
                {content}
              </button>
            )
          })}
        </div>
      </div>
    </div>
  )
}

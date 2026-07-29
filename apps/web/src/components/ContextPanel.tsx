import { Check, Copy, X } from 'lucide-react'
import { useState } from 'react'

import type { BrainStatus, Evidence } from '../types'

export function ContextPanel({
  open,
  query,
  evidence,
  selected,
  status,
  context,
  contextTokens,
  onSelect,
  onClose,
}: {
  open: boolean
  query: string
  evidence: Evidence[]
  selected: number
  status: BrainStatus | null
  context: string
  contextTokens: number
  onSelect: (index: number) => void
  onClose: () => void
}) {
  const [copied, setCopied] = useState(false)

  async function copyContext() {
    await navigator.clipboard.writeText(context)
    setCopied(true)
    window.setTimeout(() => setCopied(false), 1800)
  }

  return (
    <aside className={`context-panel ${open ? 'mobile-open' : ''}`}>
      <div className="context-heading">
        <strong>Agent context</strong>
        <button aria-label="Close agent context" onClick={onClose}>
          <X size={17} />
        </button>
      </div>
      <div className="context-scroll">
        <section className="query-summary">
          <span>Query</span>
          <p>{query}</p>
        </section>
        <section className="section-label">
          <span>Retrieved evidence</span>
          <span>{evidence.length}</span>
        </section>
        <div className="evidence-list">
          {evidence.map((item, index) => (
            <button
              key={item.chunk_id}
              className={selected === index ? 'selected' : ''}
              onClick={() => onSelect(index)}
            >
              <span>{index + 1}</span>
              <strong>{item.title}</strong>
              <time>{new Date(item.updated_at).toLocaleDateString()}</time>
            </button>
          ))}
        </div>
        <section className="provenance">
          <span className="section-title">Embedding</span>
          <p>{status?.embedding_fingerprint ?? 'unavailable'}</p>
          <p>
            {contextTokens.toLocaleString()} context tokens ·{' '}
            {(status?.embedding_cache_hits ?? 0).toLocaleString()} cache hits
          </p>
        </section>
      </div>
      <div className="copy-area">
        <button aria-label="Copy agent context" onClick={() => void copyContext()}>
          {copied ? <Check size={17} /> : <Copy size={17} />}
          {copied ? 'Context copied' : 'Copy agent context'}
        </button>
      </div>
    </aside>
  )
}

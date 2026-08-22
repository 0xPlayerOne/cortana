import { Check, Copy, LoaderCircle, RefreshCw, X } from 'lucide-react'

import type { AnswerResponse, BrainStatus, ContextBundle, Evidence } from '../types'
import { useClipboardCopy } from '../useClipboardCopy'
import { Button } from './ui/Button'

export function ContextPanel({
  open,
  query,
  evidence,
  answer,
  selected,
  status,
  context,
  contextTokens,
  serverContext,
  contextLoading,
  contextError,
  onRetrieveContext,
  onSelect,
  onClose,
}: {
  open: boolean
  query: string
  evidence: Evidence[]
  answer: AnswerResponse | null
  selected: number
  status: BrainStatus | null
  context: string
  contextTokens: number
  serverContext: ContextBundle | null
  contextLoading: boolean
  contextError: string
  onRetrieveContext: () => void
  onSelect: (index: number) => void
  onClose: () => void
}) {
  const copyValue = serverContext?.context ?? context
  const { copied, copyError, copy } = useClipboardCopy(copyValue)

  return (
    <aside className={`context-panel ${open ? 'mobile-open' : ''}`}>
      <div className="context-heading">
        <strong>Agent context</strong>
        <Button
          variant="icon"
          aria-label="Close agent context"
          data-tooltip="Close agent context"
          className="quick-tooltip"
          onClick={onClose}
        >
          <X size={17} />
        </Button>
      </div>
      <div className="context-scroll">
        <section className="query-summary">
          <span>Query</span>
          <p>{query}</p>
        </section>
        {answer && (
          <section className="retrieval-diagnostics">
            <span className="section-title">Retrieval diagnostics</span>
            <dl>
              <div>
                <dt>Mode</dt>
                <dd>{answer.mode}</dd>
              </div>
              <div>
                <dt>Latency</dt>
                <dd>{answer.cached ? 'cache hit' : `${answer.latency_ms} ms`}</dd>
              </div>
              <div>
                <dt>Planned queries</dt>
                <dd>{answer.plan.queries.length}</dd>
              </div>
              <div>
                <dt>Evidence</dt>
                <dd>{answer.evidence.length}</dd>
              </div>
            </dl>
            <ol>
              {answer.plan.queries.map((planned, index) => (
                <li key={`${planned}:${index}`}>{planned}</li>
              ))}
            </ol>
          </section>
        )}
        <section className="section-label">
          <span>Retrieved evidence</span>
          <span>{evidence.length}</span>
        </section>
        <div className="evidence-list">
          {evidence.map((item, index) => (
            <button
              type="button"
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
        {serverContext?.memories && serverContext.memories.length > 0 && (
          <>
            <section className="section-label">
              <span>Native agent memory</span>
              <span>{serverContext.memories.length}</span>
            </section>
            <div className="evidence-list">
              {serverContext.memories.map((memory) => (
                <div className="utility-item" key={memory.id}>
                  <div className="utility-item-main">
                    <strong>{memory.title}</strong>
                    <time>
                      {memory.kind} · {memory.project} · confidence {memory.confidence.toFixed(2)}
                      {memory.valid_until
                        ? ` · expires ${new Date(memory.valid_until).toLocaleDateString()}`
                        : ''}
                    </time>
                  </div>
                </div>
              ))}
            </div>
          </>
        )}
        <section className="provenance">
          <span className="section-title">Embedding</span>
          <p>{status?.embedding_fingerprint ?? 'unavailable'}</p>
          <p>
            {contextTokens.toLocaleString()} context tokens ·{' '}
            {(status?.embedding_cache_hits ?? 0).toLocaleString()} cache hits
          </p>
        </section>
        <section className="server-context">
          <span className="section-title">Agent integration bundle</span>
          <p>
            Build the exact bounded context returned by the HTTP and MCP query layer for this
            workspace scope.
          </p>
          <Button variant="secondary" disabled={contextLoading} onClick={onRetrieveContext}>
            {contextLoading ? <LoaderCircle className="spin" size={15} /> : <RefreshCw size={15} />}
            {serverContext ? 'Refresh MCP-equivalent context' : 'Build MCP-equivalent context'}
          </Button>
          {contextError && (
            <p className="context-error" role="alert">
              {contextError}
            </p>
          )}
          {serverContext && (
            <dl>
              <div>
                <dt>Included</dt>
                <dd>{serverContext.metrics.included}</dd>
              </div>
              <div>
                <dt>Omitted</dt>
                <dd>{serverContext.metrics.omitted}</dd>
              </div>
              <div>
                <dt>Tokens</dt>
                <dd>
                  {serverContext.metrics.estimated_tokens.toLocaleString()} /{' '}
                  {serverContext.metrics.max_tokens.toLocaleString()}
                </dd>
              </div>
            </dl>
          )}
        </section>
      </div>
      <div className="copy-area">
        <Button
          variant="primary"
          aria-label="Copy agent context"
          data-tooltip="Copy agent context"
          className="quick-tooltip"
          onClick={() => void copy()}
        >
          {copied ? <Check size={17} /> : <Copy size={17} />}
          {copied
            ? 'Context copied'
            : serverContext
              ? 'Copy MCP-equivalent context'
              : 'Copy preview context'}
        </Button>
        {copyError && (
          <p className="context-error" role="alert">
            {copyError}
          </p>
        )}
      </div>
    </aside>
  )
}

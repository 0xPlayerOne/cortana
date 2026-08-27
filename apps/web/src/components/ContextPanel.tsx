import { Check, Copy, LoaderCircle, RefreshCw, X } from 'lucide-react'
import { createContext, useContext } from 'react'

import type { AnswerResponse, BrainStatus, ContextBundle, Evidence } from '../types'
import { useClipboardCopy } from '../useClipboardCopy'
import { useM7SurfacePrimitives } from './m7/M7SurfacePrimitives'
import { Button, type ButtonProps } from './ui/Button'

const ContextRendererContext = createContext<'legacy' | 'shadcn'>('legacy')

function ActionButton({ variant = 'secondary', ...props }: ButtonProps) {
  const renderer = useContext(ContextRendererContext)
  const ShadcnButton = useM7SurfacePrimitives()?.Button
  if (renderer === 'legacy' || !ShadcnButton) return <Button variant={variant} {...props} />
  return (
    <ShadcnButton
      {...props}
      variant={
        variant === 'primary'
          ? 'default'
          : variant === 'danger'
            ? 'destructive'
            : variant === 'ghost' || variant === 'icon'
              ? 'ghost'
              : 'secondary'
      }
      size={variant === 'icon' ? 'icon' : variant === 'compact' ? 'sm' : 'default'}
    />
  )
}

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
  renderer = 'legacy',
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
  renderer?: 'legacy' | 'shadcn'
}) {
  const copyValue = serverContext?.context ?? context
  const { copied, copyError, copy } = useClipboardCopy(copyValue)
  const primitives = useM7SurfacePrimitives()
  const ShadcnAlert = primitives?.Alert
  const ShadcnAlertDescription = primitives?.AlertDescription
  const ShadcnBadge = primitives?.Badge
  const ShadcnCard = primitives?.Card
  const ShadcnScrollArea = primitives?.ScrollArea
  const ShadcnSpinner = primitives?.Spinner
  const QuerySection = renderer === 'shadcn' && ShadcnCard ? ShadcnCard : 'section'
  const ContextScroll = renderer === 'shadcn' && ShadcnScrollArea ? ShadcnScrollArea : 'div'

  return (
    <ContextRendererContext.Provider value={renderer}>
      <aside
        className={`context-panel ${open ? 'mobile-open' : ''} ${renderer === 'shadcn' ? 'm7-context-panel' : ''}`}
        data-m7-context-panel={renderer === 'shadcn' ? '' : undefined}
      >
        <div className="context-heading">
          <strong>Agent context</strong>
          <ActionButton
            variant="icon"
            aria-label="Close agent context"
            data-tooltip="Close agent context"
            className="quick-tooltip"
            onClick={onClose}
          >
            <X size={17} />
          </ActionButton>
        </div>
        <ContextScroll className="context-scroll">
          <QuerySection className="query-summary">
            <span>Query</span>
            <p>{query}</p>
          </QuerySection>
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
            {renderer === 'shadcn' && ShadcnBadge ? (
              <ShadcnBadge variant="secondary">{evidence.length}</ShadcnBadge>
            ) : (
              <span>{evidence.length}</span>
            )}
          </section>
          <div className="evidence-list">
            {evidence.map((item, index) => (
              <ActionButton
                variant="ghost"
                type="button"
                key={item.chunk_id}
                className={selected === index ? 'selected' : ''}
                onClick={() => onSelect(index)}
              >
                <span>{index + 1}</span>
                <strong>{item.title}</strong>
                <time>{new Date(item.updated_at).toLocaleDateString()}</time>
              </ActionButton>
            ))}
          </div>
          {serverContext?.memories && serverContext.memories.length > 0 && (
            <>
              <section className="section-label">
                <span>Native agent memory</span>
                {renderer === 'shadcn' && ShadcnBadge ? (
                  <ShadcnBadge variant="secondary">{serverContext.memories.length}</ShadcnBadge>
                ) : (
                  <span>{serverContext.memories.length}</span>
                )}
              </section>
              <div className="evidence-list">
                {serverContext.memories.map((memory) => (
                  <div className="utility-item" key={memory.id}>
                    <div className="utility-item-main">
                      <strong>{memory.title}</strong>
                      <time>
                        {memory.content_type ?? memory.kind} · {memory.retention_tier ?? 'durable'}{' '}
                        · {memory.scope ?? 'workspace'} · {memory.project} · confidence{' '}
                        {memory.confidence.toFixed(2)}
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
          {serverContext?.degradation && (
            <p className="context-error" role="status">
              Degraded retrieval:{' '}
              {serverContext.degradation.detail || serverContext.degradation.code}
            </p>
          )}
          <section className="provenance">
            <span className="section-title">Embedding</span>
            <p>{status?.embedding_fingerprint ?? 'unavailable'}</p>
            {serverContext?.context_bundle_id && (
              <p>Bundle {serverContext.context_bundle_id.slice(0, 16)}…</p>
            )}
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
            <ActionButton variant="secondary" disabled={contextLoading} onClick={onRetrieveContext}>
              {contextLoading ? (
                renderer === 'shadcn' && ShadcnSpinner ? (
                  <ShadcnSpinner />
                ) : (
                  <LoaderCircle className="spin" size={15} />
                )
              ) : (
                <RefreshCw size={15} />
              )}
              {serverContext ? 'Refresh MCP-equivalent context' : 'Build MCP-equivalent context'}
            </ActionButton>
            {contextError &&
              (renderer === 'shadcn' && ShadcnAlert && ShadcnAlertDescription ? (
                <ShadcnAlert variant="destructive">
                  <ShadcnAlertDescription>{contextError}</ShadcnAlertDescription>
                </ShadcnAlert>
              ) : (
                <p className="context-error" role="alert">
                  {contextError}
                </p>
              ))}
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
        </ContextScroll>
        <div className="copy-area">
          <ActionButton
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
          </ActionButton>
          {copyError &&
            (renderer === 'shadcn' && ShadcnAlert && ShadcnAlertDescription ? (
              <ShadcnAlert variant="destructive">
                <ShadcnAlertDescription>{copyError}</ShadcnAlertDescription>
              </ShadcnAlert>
            ) : (
              <p className="context-error" role="alert">
                {copyError}
              </p>
            ))}
        </div>
      </aside>
    </ContextRendererContext.Provider>
  )
}

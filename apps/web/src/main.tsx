import { lazy, StrictMode, Suspense } from 'react'
import { createRoot } from 'react-dom/client'

import { RendererErrorBoundary } from './components/RendererErrorBoundary'
import { resolveRendererMode } from './rendererMode'
import { applyTheme, readThemePreference } from './theme'

const LegacyRenderer = lazy(() =>
  import('./LegacyRenderer').then((module) => ({ default: module.LegacyRenderer }))
)
const ShadcnRenderer = lazy(() =>
  import('./components/m7/M7ShadcnPrototype').then((module) => ({
    default: module.M7ShadcnPrototype,
  }))
)

const env = (import.meta as ImportMeta & { env?: Record<string, unknown> }).env
const buildRenderer = env?.['VITE_CORTANA_RENDERER']
const renderer = resolveRendererMode({
  search: window.location.search,
  buildRenderer: typeof buildRenderer === 'string' ? buildRenderer : undefined,
  allowQueryOverride: String(env?.['DEV']) === 'true',
})

if (renderer === 'shadcn') applyTheme(readThemePreference())

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <RendererErrorBoundary>
      <Suspense fallback={<main aria-label="Loading Cortana" />}>
        {renderer === 'shadcn' ? <ShadcnRenderer /> : <LegacyRenderer />}
      </Suspense>
    </RendererErrorBoundary>
  </StrictMode>
)

import { lazy, StrictMode, Suspense } from 'react'
import { createRoot } from 'react-dom/client'

import { RendererErrorBoundary } from './components/RendererErrorBoundary'
import { applyTheme, DEFAULT_THEME } from './theme'

const App = lazy(() =>
  import('./App').then((module) => ({
    default: module.App,
  }))
)

applyTheme(DEFAULT_THEME)

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <RendererErrorBoundary>
      <Suspense fallback={<main aria-label="Loading Cortana" />}>
        <App />
      </Suspense>
    </RendererErrorBoundary>
  </StrictMode>
)

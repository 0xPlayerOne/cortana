import { lazy, StrictMode, Suspense } from 'react'
import { createRoot } from 'react-dom/client'

import { RendererErrorBoundary } from './components/RendererErrorBoundary'
import { applyTheme, readThemePreference } from './theme'

const App = lazy(() =>
  import('./App').then((module) => ({
    default: module.App,
  }))
)

applyTheme(readThemePreference())

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <RendererErrorBoundary>
      <Suspense fallback={<main aria-label="Loading Cortana" />}>
        <App />
      </Suspense>
    </RendererErrorBoundary>
  </StrictMode>
)

import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import { App } from './App'
import { AppErrorBoundary } from './components/AppErrorBoundary'
import { resolveRendererMode } from './rendererMode'
import { applyTheme, readThemePreference } from './theme'
import './styles.css'

const env = (import.meta as ImportMeta & { env?: Record<string, unknown> }).env
const buildRenderer = env?.['VITE_CORTANA_RENDERER']
const renderer = resolveRendererMode({
  search: window.location.search,
  buildRenderer: typeof buildRenderer === 'string' ? buildRenderer : undefined,
  allowQueryOverride: String(env?.['DEV']) === 'true',
})

async function render() {
  const root = createRoot(document.getElementById('root')!)
  let content = <App />

  if (renderer === 'shadcn') {
    applyTheme(readThemePreference())
    await import('./shadcn.css')
    const { M7ShadcnPrototype } = await import('./components/m7/M7ShadcnPrototype')
    content = <M7ShadcnPrototype />
  }

  root.render(
    <StrictMode>
      <AppErrorBoundary>{content}</AppErrorBoundary>
    </StrictMode>
  )
}

void render()

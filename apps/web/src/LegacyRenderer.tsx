import { App } from './App'
import { AppErrorBoundary } from './components/AppErrorBoundary'
import './styles.css'

export function LegacyRenderer() {
  return (
    <AppErrorBoundary>
      <App />
    </AppErrorBoundary>
  )
}

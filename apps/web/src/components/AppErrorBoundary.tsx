import { Component, type ErrorInfo, type ReactNode } from 'react'
import { AlertTriangle } from 'lucide-react'

type Props = { children: ReactNode }
type State = { failed: boolean }

/** Keeps a renderer exception from leaving the desktop window blank. */
export class AppErrorBoundary extends Component<Props, State> {
  state: State = { failed: false }

  static getDerivedStateFromError(): State {
    return { failed: true }
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('Cortana renderer failed', error, info.componentStack)
  }

  render() {
    if (!this.state.failed) return this.props.children
    return (
      <main className="empty-state runtime-error" role="alert">
        <AlertTriangle size={30} />
        <h1>Cortana needs a reload</h1>
        <p>
          The workspace hit an unexpected renderer error. Your local index and settings are safe.
        </p>
        <button type="button" onClick={() => window.location.reload()}>
          Reload workspace
        </button>
      </main>
    )
  }
}

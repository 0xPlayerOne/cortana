import { Component, type ErrorInfo, type ReactNode } from 'react'

type Props = { children: ReactNode }
type State = { failed: boolean }

/** Catches renderer chunk and root failures before either visual system loads. */
export class RendererErrorBoundary extends Component<Props, State> {
  override state: State = { failed: false }

  static getDerivedStateFromError(): State {
    return { failed: true }
  }

  override componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('Cortana renderer failed', error, info.componentStack)
  }

  override render() {
    if (!this.state.failed) return this.props.children
    return (
      <main
        role="alert"
        style={{
          alignItems: 'center',
          background: 'var(--background, #0f1624)',
          color: 'var(--foreground, #f0f3fc)',
          display: 'flex',
          flexDirection: 'column',
          fontFamily: 'system-ui, sans-serif',
          gap: '0.75rem',
          justifyContent: 'center',
          minHeight: '100vh',
          padding: '2rem',
          textAlign: 'center',
        }}
      >
        <h1>Cortana needs a reload</h1>
        <p>The renderer could not load. Your local index and settings are safe.</p>
        <button
          type="button"
          onClick={() => window.location.reload()}
          style={{
            background: 'var(--primary, #59defc)',
            border: 0,
            borderRadius: '0.5rem',
            color: 'var(--primary-foreground, #151b2b)',
            cursor: 'pointer',
            font: 'inherit',
            fontWeight: 600,
            minHeight: '2.75rem',
            padding: '0.625rem 1rem',
          }}
        >
          Reload workspace
        </button>
      </main>
    )
  }
}

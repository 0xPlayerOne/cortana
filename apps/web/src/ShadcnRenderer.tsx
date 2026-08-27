import { App } from './App'
import { AppErrorBoundary } from './components/AppErrorBoundary'
import { M7ActivityInbox } from './components/m7/M7ActivityInbox'
import {
  M7ApplicationHeader,
  M7ApplicationNavigation,
  M7CommandPalette,
  M7PanelBoundary,
  M7ShellProvider,
  M7StatusBar,
} from './components/m7/M7ApplicationShell'
import { m7SurfacePrimitives } from './components/m7/M7SurfacePrimitives.shadcn'
import { M7SurfacePrimitivesProvider } from './components/m7/M7SurfacePrimitives'
import './styles.css'
import './shadcn.css'

const m7ShellComponents = {
  ActivityInbox: M7ActivityInbox,
  ApplicationHeader: M7ApplicationHeader,
  ApplicationNavigation: M7ApplicationNavigation,
  CommandPalette: M7CommandPalette,
  PanelBoundary: M7PanelBoundary,
  ShellProvider: M7ShellProvider,
  StatusBar: M7StatusBar,
}

export function ShadcnRenderer() {
  return (
    <AppErrorBoundary>
      <M7SurfacePrimitivesProvider value={m7SurfacePrimitives}>
        <App renderer="shadcn" shadcnShell={m7ShellComponents} />
      </M7SurfacePrimitivesProvider>
    </AppErrorBoundary>
  )
}

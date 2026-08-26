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
      <App renderer="shadcn" shadcnShell={m7ShellComponents} />
    </AppErrorBoundary>
  )
}

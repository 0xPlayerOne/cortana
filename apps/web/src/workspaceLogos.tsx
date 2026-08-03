import { useEffect, useState } from 'react'

import type { WorkspaceSettings } from './types'
import { LOGO_EVENT, readWorkspaceLogo } from './workspaceLogoStore'

export function WorkspaceLogo({
  workspace,
  size = 'medium',
}: {
  workspace: Pick<WorkspaceSettings, 'id' | 'name' | 'color'>
  size?: 'small' | 'medium' | 'large'
}) {
  const [logo, setLogo] = useState(() => readWorkspaceLogo(workspace.id))

  useEffect(() => {
    const refresh = () => setLogo(readWorkspaceLogo(workspace.id))
    refresh()
    window.addEventListener(LOGO_EVENT, refresh)
    return () => window.removeEventListener(LOGO_EVENT, refresh)
  }, [workspace.id])

  if (logo) {
    return (
      <img
        className={`workspace-logo workspace-logo--${size}${size === 'small' ? ' workspace-picker-mark' : ''}`}
        src={logo}
        alt=""
        aria-hidden="true"
      />
    )
  }

  return (
    <span
      className={`workspace-logo workspace-logo--${size}${size === 'small' ? ' workspace-picker-mark' : ''}`}
      style={{ background: workspace.color || 'var(--amber)' }}
      aria-hidden="true"
    >
      {(workspace.name.trim()[0] || '?').toUpperCase()}
    </span>
  )
}

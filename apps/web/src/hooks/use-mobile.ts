import * as React from 'react'

// Keep the compact shell through the 768px tablet acceptance width. The
// desktop sidebar needs enough room to coexist with the bounded source and
// workspace panes, so it begins at 800px instead of switching at 768px.
const MOBILE_BREAKPOINT = 800

export function useIsMobile() {
  const [isMobile, setIsMobile] = React.useState<boolean | undefined>(undefined)

  React.useEffect(() => {
    const mql = window.matchMedia(`(max-width: ${MOBILE_BREAKPOINT - 1}px)`)
    const onChange = () => {
      setIsMobile(window.innerWidth < MOBILE_BREAKPOINT)
    }
    mql.addEventListener('change', onChange)
    setIsMobile(window.innerWidth < MOBILE_BREAKPOINT)
    return () => mql.removeEventListener('change', onChange)
  }, [])

  return !!isMobile
}

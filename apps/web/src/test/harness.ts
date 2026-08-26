import { GlobalRegistrator } from '@happy-dom/global-registrator'

// Mocked browser environment for React component tests, registered before any
// test file (and therefore before @testing-library/react) evaluates.
GlobalRegistrator.register()

// Base UI's ScrollArea waits for subtree animations before measuring its
// thumb. happy-dom does not implement the Web Animations API yet, so expose
// the browser-compatible empty result used when no animations are active.
if (!Element.prototype.getAnimations) {
  Element.prototype.getAnimations = () => []
}

const { configure } = await import('@testing-library/dom')

// Cold Bun workers and the first happy-dom render can exceed Testing
// Library's one-second polling default on developer machines. Keep the test
// harness bounded, but leave enough time for a legitimate initial render when
// Code Foundry is running independent suites in parallel.
configure({ asyncUtilTimeout: 10_000 })

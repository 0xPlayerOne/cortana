import { GlobalRegistrator } from '@happy-dom/global-registrator'

// Mocked browser environment for React component tests, registered before any
// test file (and therefore before @testing-library/react) evaluates.
GlobalRegistrator.register()

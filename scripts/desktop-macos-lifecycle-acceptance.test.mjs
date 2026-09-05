import { spawnSync } from 'node:child_process'

import { expect, test } from 'bun:test'

import {
  APPLESCRIPT_PROBE,
  APPLESCRIPT_TRAY_CYCLE,
  SWIFT_SCREEN_WINDOW_PROBE,
  buildLifecycleEvidence,
  parseProbeOutput,
  parseScreenWindowProbeOutput,
  parseTrayCycleOutput,
  buildMacosLaunch,
  resolveMacosBundlePath,
  summarizeTrayMenu,
} from './desktop-macos-lifecycle-acceptance.mjs'

test('macOS lifecycle probe parsing keeps native menu evidence bounded to booleans', () => {
  const probe = parseProbeOutput(
    'window=true\ntray=true\nmenu=Runtime: online\tCorpus: 12 docs · 30 chunks\tIngestion: manual\tSource jobs: idle\tShow Cortana\tQuit Cortana Desktop'
  )
  expect(probe).toEqual({
    windowPresent: true,
    trayPresent: true,
    menuLabels: [
      'Runtime: online',
      'Corpus: 12 docs · 30 chunks',
      'Ingestion: manual',
      'Source jobs: idle',
      'Show Cortana',
      'Quit Cortana Desktop',
    ],
  })
  expect(summarizeTrayMenu(probe.menuLabels)).toEqual({
    runtimeStatus: true,
    corpusStatus: true,
    ingestionStatus: true,
    sourceJobsStatus: true,
    show: true,
    quit: true,
  })
})

test('macOS lifecycle screen diagnostic is bounded and fails closed when unavailable', () => {
  expect(SWIFT_SCREEN_WINDOW_PROBE).toContain('CoreGraphics')
  expect(parseScreenWindowProbeOutput('screen_window=true\n')).toBe(true)
  expect(parseScreenWindowProbeOutput('screen_window=false\n')).toBe(false)
  expect(parseScreenWindowProbeOutput('diagnostic-unavailable\n')).toBeNull()
})

test('macOS lifecycle launches a packaged app through LaunchServices', () => {
  const executable = '/runner/work/Cortana.app/Contents/MacOS/cortana-desktop'
  const bundle = '/runner/work/Cortana.app'

  expect(resolveMacosBundlePath(executable)).toBe(bundle)
  expect(buildMacosLaunch(executable)).toEqual({
    command: 'open',
    args: ['-n', bundle],
    mode: 'launch-services',
  })
  expect(buildMacosLaunch('/runner/work/cortana-desktop')).toEqual({
    command: '/runner/work/cortana-desktop',
    args: [],
    mode: 'executable',
  })
})

test('macOS lifecycle AppleScript constants contain valid continuation tokens', () => {
  expect(APPLESCRIPT_PROBE).toContain('¬')
  expect(APPLESCRIPT_TRAY_CYCLE).toContain('¬')
  expect(APPLESCRIPT_PROBE).not.toContain('\\u00AC')
  expect(APPLESCRIPT_TRAY_CYCLE).not.toContain('\\u00AC')

  if (process.platform === 'darwin') {
    for (const script of [APPLESCRIPT_PROBE, APPLESCRIPT_TRAY_CYCLE]) {
      const result = spawnSync('osascript', ['-e', script], { encoding: 'utf8' })
      expect(result.status).toBe(0)
      expect(result.stderr).not.toContain('syntax error')
    }
  }
}, 15_000)

test('macOS lifecycle evidence requires a hidden window after close and a reopened window', () => {
  const cycle = parseTrayCycleOutput('process=true\nwindow_after_close=false\ntray_reopen=true\n')
  const evidence = buildLifecycleEvidence({
    version: '0.56.3',
    app: '/runner/work/Cortana.app/Contents/MacOS/cortana-desktop',
    startupMs: 1234,
    firstRun: {
      no_implicit_connector_install: true,
      query_only_default: true,
      no_implicit_side_effects: true,
    },
    probe: {
      windowPresent: true,
      trayPresent: true,
      menuLabels: [
        'Runtime: online',
        'Corpus: 12 docs · 30 chunks',
        'Ingestion: manual',
        'Source jobs: idle',
        'Show Cortana',
        'Quit Cortana Desktop',
      ],
    },
    cycle,
  })
  expect(evidence).toMatchObject({
    status: 'passed',
    installation_type: 'published-package-macos-native-lifecycle',
    host: { status: 'passed', isolated_state: true },
    window: { initial: 'present', after_close: 'hidden', after_show: 'present' },
  })
  expect(JSON.stringify(evidence)).not.toContain('12 docs')
})

test('macOS lifecycle evidence fails closed when tray controls are incomplete', () => {
  const evidence = buildLifecycleEvidence({
    version: '0.56.3',
    app: '/runner/Cortana.app/Contents/MacOS/cortana-desktop',
    startupMs: 1,
    firstRun: {
      no_implicit_connector_install: true,
      query_only_default: true,
      no_implicit_side_effects: true,
    },
    probe: { windowPresent: true, trayPresent: false, menuLabels: [] },
    cycle: { processRunning: true, windowAfterClose: false, trayReopen: false },
  })
  expect(evidence.status).toBe('failed')
})

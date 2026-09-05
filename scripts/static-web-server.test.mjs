import { spawn } from 'node:child_process'
import { mkdtempSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { get as httpGet } from 'node:http'
import { fileURLToPath } from 'node:url'
import { join } from 'node:path'
import { tmpdir } from 'node:os'

import { expect, test } from 'bun:test'

import {
  assertStaticFileInsideRoot,
  contentTypeForPath,
  resolveStaticFilePath,
} from './static-web-server.mjs'

const SERVER_SCRIPT = fileURLToPath(new URL('./static-web-server.mjs', import.meta.url))

test('static web server resolves only files inside the packaged web root', () => {
  const root = '/runner/temp/cortana-web'
  expect(resolveStaticFilePath(root, '/')).toBe(`${root}/index.html`)
  expect(resolveStaticFilePath(root, '/assets/app.js')).toBe(`${root}/assets/app.js`)
  expect(() => resolveStaticFilePath(root, '/%2e%2e/secrets.txt')).toThrow(
    'static request escapes web root'
  )
})

test('static web server reports browser content types for packaged assets', () => {
  expect(contentTypeForPath('index.html')).toBe('text/html; charset=utf-8')
  expect(contentTypeForPath('assets/app.js')).toBe('text/javascript; charset=utf-8')
  expect(contentTypeForPath('assets/app.css')).toBe('text/css; charset=utf-8')
  expect(contentTypeForPath('assets/font.woff2')).toBe('font/woff2')
  expect(contentTypeForPath('assets/unknown.bin')).toBe('application/octet-stream')
})

test('static web server rejects symlink escapes from the packaged web root', () => {
  const root = mkdtempSync(join(tmpdir(), 'cortana-static-web-root-'))
  const outside = mkdtempSync(join(tmpdir(), 'cortana-static-web-outside-'))
  try {
    writeFileSync(join(outside, 'secrets.txt'), 'secret')
    const link = join(root, 'link.txt')
    symlinkSync(join(outside, 'secrets.txt'), link)
    expect(() => assertStaticFileInsideRoot(root, link)).toThrow('static request escapes web root')
    writeFileSync(join(root, 'index.html'), '<!doctype html>')
    expect(assertStaticFileInsideRoot(root, join(root, 'index.html'))).toBe(
      realpathSync(join(root, 'index.html'))
    )
  } finally {
    rmSync(root, { recursive: true, force: true })
    rmSync(outside, { recursive: true, force: true })
  }
})

test('static web server serves a packaged index on an ephemeral loopback port', async () => {
  const root = mkdtempSync(join(tmpdir(), 'cortana-static-web-test-'))
  const child = spawn(process.execPath, [SERVER_SCRIPT, '--directory', root, '--port', '0'], {
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  try {
    writeFileSync(join(root, 'index.html'), '<!doctype html><title>packaged</title>')
    const address = await new Promise((resolveAddress, reject) => {
      let output = ''
      const onData = (chunk) => {
        output += chunk.toString()
        const match = output.match(/listening on http:\/\/127\.0\.0\.1:(\d+)/)
        if (match) resolveAddress(`http://127.0.0.1:${match[1]}`)
      }
      child.stdout.on('data', onData)
      child.once('error', reject)
      child.once('exit', (code) => reject(new Error(`static server exited: ${code}`)))
    })
    const response = await new Promise((resolveResponse, reject) => {
      httpGet(address, (response) => {
        let body = ''
        response.setEncoding('utf8')
        response.on('data', (chunk) => {
          body += chunk
        })
        response.on('end', () => resolveResponse({ status: response.statusCode, body }))
      }).on('error', reject)
    })
    expect(response).toEqual({ status: 200, body: '<!doctype html><title>packaged</title>' })
  } finally {
    if (child.exitCode === null) {
      child.kill('SIGTERM')
      await new Promise((resolveExit) => child.once('exit', resolveExit))
    }
    rmSync(root, { recursive: true, force: true })
  }
})

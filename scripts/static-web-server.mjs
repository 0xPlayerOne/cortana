#!/usr/bin/env node

import { createReadStream, existsSync, realpathSync, statSync } from 'node:fs'
import { createServer } from 'node:http'
import { extname, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'

const CONTENT_TYPES = Object.freeze({
  '.css': 'text/css; charset=utf-8',
  '.gif': 'image/gif',
  '.html': 'text/html; charset=utf-8',
  '.ico': 'image/x-icon',
  '.jpeg': 'image/jpeg',
  '.jpg': 'image/jpeg',
  '.js': 'text/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.map': 'application/json; charset=utf-8',
  '.png': 'image/png',
  '.svg': 'image/svg+xml',
  '.wasm': 'application/wasm',
  '.webp': 'image/webp',
  '.woff': 'font/woff',
  '.woff2': 'font/woff2',
})

export function contentTypeForPath(path) {
  return CONTENT_TYPES[extname(path).toLowerCase()] || 'application/octet-stream'
}

export function resolveStaticFilePath(directory, requestPath) {
  const root = resolve(directory)
  const queryStart = requestPath.search(/[?#]/)
  const rawPathname = queryStart === -1 ? requestPath : requestPath.slice(0, queryStart)
  const pathname = decodeURIComponent(rawPathname || '/')
  const requestedPath = pathname === '/' ? 'index.html' : pathname.replace(/^\/+/, '')
  const filePath = resolve(root, requestedPath)
  const rootPrefix = root.endsWith(sep) ? root : `${root}${sep}`
  if (filePath !== root && !filePath.startsWith(rootPrefix)) {
    throw new Error('static request escapes web root')
  }
  return filePath
}

export function assertStaticFileInsideRoot(root, filePath) {
  const realRoot = realpathSync(root)
  const realPath = realpathSync(filePath)
  const realPrefix = realRoot.endsWith(sep) ? realRoot : `${realRoot}${sep}`
  if (realPath !== realRoot && !realPath.startsWith(realPrefix)) {
    throw new Error('static request escapes web root')
  }
  return realPath
}

export function createStaticWebServer({ directory, address = '127.0.0.1', port = 0 }) {
  if (!directory || !existsSync(directory) || !statSync(directory).isDirectory()) {
    throw new Error(`static web directory does not exist: ${directory}`)
  }
  const root = resolve(directory)
  return createServer((request, response) => {
    if (request.method !== 'GET' && request.method !== 'HEAD') {
      response.writeHead(405, { allow: 'GET, HEAD' })
      response.end()
      return
    }
    let filePath
    try {
      filePath = resolveStaticFilePath(root, request.url || '/')
    } catch {
      response.writeHead(400)
      response.end('invalid static request')
      return
    }
    let fileStats
    try {
      fileStats = statSync(filePath)
    } catch {
      response.writeHead(404)
      response.end('not found')
      return
    }
    if (!fileStats.isFile()) {
      response.writeHead(404)
      response.end('not found')
      return
    }
    try {
      assertStaticFileInsideRoot(root, filePath)
    } catch {
      response.writeHead(400)
      response.end('invalid static request')
      return
    }
    response.writeHead(200, {
      'cache-control': 'no-store',
      'content-length': fileStats.size,
      'content-type': contentTypeForPath(filePath),
    })
    if (request.method === 'HEAD') {
      response.end()
      return
    }
    createReadStream(filePath)
      .on('error', () => response.destroy())
      .pipe(response)
  }).listen(port, address)
}

function parseArguments(args) {
  const values = {}
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index]
    if (!argument.startsWith('--')) throw new Error(`unexpected argument: ${argument}`)
    const key = argument.slice(2)
    const value = args[index + 1]
    if (!value || value.startsWith('--')) throw new Error(`missing value for --${key}`)
    values[key] = value
    index += 1
  }
  return values
}

function main(args = process.argv.slice(2)) {
  const values = parseArguments(args)
  const directory = values.directory
  const address = values.address || '127.0.0.1'
  const port = Number(values.port || 0)
  if (!directory || !Number.isInteger(port) || port < 0 || port > 65_535) {
    throw new Error(
      'usage: static-web-server.mjs --directory DIR [--address ADDRESS] [--port PORT]'
    )
  }
  const server = createStaticWebServer({ directory, address, port })
  const announce = () => {
    const listener = server.address()
    const actualPort = typeof listener === 'object' && listener ? listener.port : port
    console.log(`static web server listening on http://${address}:${actualPort}`)
  }
  if (server.listening) announce()
  else server.once('listening', announce)
  const shutdown = () => server.close(() => process.exit(0))
  process.once('SIGINT', shutdown)
  process.once('SIGTERM', shutdown)
  return server
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main()
  } catch (error) {
    console.error(error instanceof Error ? error.message : error)
    process.exitCode = 1
  }
}

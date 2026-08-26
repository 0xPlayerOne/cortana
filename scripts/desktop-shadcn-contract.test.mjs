import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const root = resolve(import.meta.dir, '..')

describe('Desktop shadcn renderer contract', () => {
  test('pins the official Base UI Nova preset and semantic Tailwind entrypoint', () => {
    const config = JSON.parse(readFileSync(resolve(root, 'apps/web/components.json'), 'utf8'))

    expect(config).toMatchObject({
      style: 'base-nova',
      rsc: false,
      tsx: true,
      iconLibrary: 'lucide',
      tailwind: {
        css: 'src/shadcn.css',
        cssVariables: true,
      },
      aliases: {
        components: '@/components',
        ui: '@/components/shadcn',
        hooks: '@/hooks',
        lib: '@/lib',
        utils: '@/lib/utils',
      },
    })
  })

  test('keeps Tailwind and the Vite plugin in the web workspace', () => {
    const manifest = JSON.parse(readFileSync(resolve(root, 'apps/web/package.json'), 'utf8'))

    expect(manifest.devDependencies).toMatchObject({
      '@tailwindcss/vite': expect.any(String),
      tailwindcss: expect.any(String),
    })
  })

  test('resolves generated components through the checked-in source alias', () => {
    const tsconfig = JSON.parse(readFileSync(resolve(root, 'apps/web/tsconfig.json'), 'utf8'))

    expect(tsconfig.compilerOptions).toMatchObject({
      baseUrl: '.',
      paths: { '@/*': ['./src/*'] },
    })
  })

  test('disables renderer motion when the operating system requests it', () => {
    const css = readFileSync(resolve(root, 'apps/web/src/shadcn.css'), 'utf8')

    expect(css).toContain('@media (prefers-reduced-motion: reduce)')
    expect(css).toContain('animation-duration: 0.01ms !important')
    expect(css).toContain('transition-duration: 0.01ms !important')
  })
})

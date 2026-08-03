import {
  Bot,
  CalendarDays,
  Cloud,
  Code2,
  Database,
  Folder,
  Mail,
  MessageCircle,
  StickyNote,
} from 'lucide-react'
import { siApple, siDiscord, siGmail, siGoogledrive, siGooglecalendar } from 'simple-icons'
import type { SimpleIcon } from 'simple-icons'

type BrandGlyph = Pick<SimpleIcon, 'path' | 'hex'>

const sourceIcons: Record<string, typeof Folder> = {
  filesystem: Code2,
  'google-drive': Cloud,
  'google-calendar': CalendarDays,
  gmail: Mail,
  'apple-notes': StickyNote,
  discord: MessageCircle,
  slack: MessageCircle,
  buzz: Bot,
}

const sourceBrands: Record<string, BrandGlyph> = {
  'apple-notes': siApple,
  discord: siDiscord,
  gmail: siGmail,
  'google-drive': siGoogledrive,
  'google-calendar': siGooglecalendar,
  // Slack was removed from the current Simple Icons release. Keep the
  // official CC0-era path local so the connector still gets a real brand
  // mark without pulling a second icon package into the bundle.
  slack: {
    hex: '4A154B',
    path: 'M5.042 15.165a2.528 2.528 0 0 1-2.52 2.523A2.528 2.528 0 0 1 0 15.165a2.527 2.527 0 0 1 2.522-2.52h2.52v2.52zM6.313 15.165a2.527 2.527 0 0 1 2.521-2.52 2.527 2.527 0 0 1 2.521 2.52v6.313A2.528 2.528 0 0 1 8.834 24a2.528 2.528 0 0 1-2.521-2.522v-6.313zM8.834 5.042a2.528 2.528 0 0 1-2.521-2.52A2.528 2.528 0 0 1 8.834 0a2.528 2.528 0 0 1 2.521 2.522v2.52H8.834zM8.834 6.313a2.528 2.528 0 0 1 2.521 2.521 2.528 2.528 0 0 1-2.521 2.521H2.522A2.528 2.528 0 0 1 0 8.834a2.528 2.528 0 0 1 2.522-2.521h6.312zM18.956 8.834a2.528 2.528 0 0 1 2.522-2.521A2.528 2.528 0 0 1 24 8.834a2.528 2.528 0 0 1-2.522 2.521h-2.522V8.834zM17.688 8.834a2.528 2.528 0 0 1-2.523 2.521 2.527 2.527 0 0 1-2.52-2.521V2.522A2.527 2.527 0 0 1 15.165 0a2.528 2.528 0 0 1 2.523 2.522v6.312zM15.165 18.956a2.528 2.528 0 0 1 2.523 2.522A2.527 2.527 0 0 1 15.165 24a2.527 2.527 0 0 1-2.52-2.522v-2.522h2.52zM15.165 17.688a2.527 2.527 0 0 1-2.52-2.523 2.526 2.526 0 0 1 2.52-2.52h6.313A2.527 2.527 0 0 1 24 15.165a2.528 2.528 0 0 1-2.522 2.523h-6.313z',
  },
}

export function sourceIconForKind(kind: string) {
  return sourceIcons[kind] || Database
}

export function sourceBrandForKind(kind: string): BrandGlyph | undefined {
  return sourceBrands[kind]
}

/** Stable human-facing labels; raw source IDs remain available in Advanced settings. */
export function sourceDisplayName(kind: string, fallback: string): string {
  const labels: Record<string, string> = {
    filesystem: 'Files & code',
    'apple-notes': 'Apple Notes',
    buzz: 'Buzz',
    'google-drive': 'Google Drive',
    gmail: 'Gmail',
    'google-calendar': 'Google Calendar',
    slack: 'Slack',
    discord: 'Discord',
  }
  return labels[kind] ?? (kind === 'indexed' ? fallback : 'External source')
}

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

export function sourceIconForKind(kind: string) {
  return sourceIcons[kind] || Database
}

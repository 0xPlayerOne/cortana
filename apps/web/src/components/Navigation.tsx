import {
  ArrowLeft,
  ArrowRight,
  BookOpenText,
  CalendarDays,
  CircleHelp,
  Clock3,
  Database,
  Filter,
  GitFork,
  Inbox,
  Menu,
  MessageCircle,
  PanelRightClose,
  Search,
  Settings,
  Sparkles,
  TerminalSquare,
} from 'lucide-react'

export function TitleActions({
  context = false,
  onOpenSources,
  onOpenContext,
}: {
  context?: boolean
  onOpenSources?: () => void
  onOpenContext?: () => void
}) {
  if (context) {
    return (
      <div className="title-actions">
        <button aria-label="Filter results">
          <Filter size={18} />
        </button>
        <button aria-label="Search history">
          <Clock3 size={18} />
        </button>
        <button className="mobile-button" aria-label="Open agent context" onClick={onOpenContext}>
          <PanelRightClose size={18} />
        </button>
      </div>
    )
  }
  return (
    <>
      <button className="mobile-button" aria-label="Open sources" onClick={onOpenSources}>
        <Menu size={19} />
      </button>
      <div className="history-buttons" aria-hidden="true">
        <ArrowLeft size={18} />
        <ArrowRight size={18} />
      </div>
    </>
  )
}

export function Navigation() {
  return (
    <nav className="rail" aria-label="Primary">
      <div className="brand-mark" aria-label="Cortana">
        <Sparkles size={24} />
      </div>
      <RailButton icon={Search} label="Search" />
      <RailButton icon={BookOpenText} label="Knowledge" active />
      <RailButton icon={GitFork} label="Graph" />
      <RailButton icon={Inbox} label="Inbox" />
      <RailButton icon={MessageCircle} label="Conversations" />
      <RailButton icon={TerminalSquare} label="Agent tools" />
      <RailButton icon={CalendarDays} label="Timeline" />
      <RailButton icon={Database} label="Index" />
      <div className="rail-spacer" />
      <RailButton icon={Settings} label="Settings" />
      <RailButton icon={CircleHelp} label="Help" />
      <div className="avatar">AC</div>
    </nav>
  )
}

function RailButton({
  icon: Icon,
  label,
  active = false,
}: {
  icon: typeof Search
  label: string
  active?: boolean
}) {
  return (
    <button className={`rail-button ${active ? 'active' : ''}`} aria-label={label} title={label}>
      <Icon size={20} />
    </button>
  )
}

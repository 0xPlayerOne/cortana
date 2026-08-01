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
        <button aria-label="Filter results" title="Result filters are coming later" disabled>
          <Filter size={18} />
        </button>
        <button aria-label="Search history" title="Search history is coming later" disabled>
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

export type AppView =
  'knowledge' | 'settings' | 'inbox' | 'conversations' | 'agent-tools' | 'index' | 'help'

export function Navigation({
  view,
  onNavigate,
  onSearch,
  onOpenGraph,
  onOpenTimeline,
}: {
  view: AppView
  onNavigate: (view: AppView) => void
  onSearch: () => void
  onOpenGraph: () => void
  onOpenTimeline: () => void
}) {
  return (
    <nav className="rail" aria-label="Primary">
      <div className="brand-mark" aria-label="Cortana">
        <Sparkles size={24} />
      </div>
      <RailButton icon={Search} label="Search" onClick={onSearch} />
      <RailButton
        icon={BookOpenText}
        label="Knowledge"
        active={view === 'knowledge'}
        onClick={() => onNavigate('knowledge')}
      />
      <RailButton icon={GitFork} label="Graph" onClick={onOpenGraph} />
      <RailButton
        icon={Inbox}
        label="Inbox"
        active={view === 'inbox'}
        onClick={() => onNavigate('inbox')}
      />
      <RailButton
        icon={MessageCircle}
        label="Conversations"
        active={view === 'conversations'}
        onClick={() => onNavigate('conversations')}
      />
      <RailButton
        icon={TerminalSquare}
        label="Agent tools"
        active={view === 'agent-tools'}
        onClick={() => onNavigate('agent-tools')}
      />
      <RailButton icon={CalendarDays} label="Timeline" onClick={onOpenTimeline} />
      <RailButton
        icon={Database}
        label="Index"
        active={view === 'index'}
        onClick={() => onNavigate('index')}
      />
      <div className="rail-spacer" />
      <RailButton
        icon={Settings}
        label="Settings"
        active={view === 'settings'}
        onClick={() => onNavigate('settings')}
      />
      <RailButton
        icon={CircleHelp}
        label="Help"
        active={view === 'help'}
        onClick={() => onNavigate('help')}
      />
      <div className="avatar">AC</div>
    </nav>
  )
}

function RailButton({
  icon: Icon,
  label,
  active = false,
  onClick,
}: {
  icon: typeof Search
  label: string
  active?: boolean
  onClick: () => void
}) {
  return (
    <button
      className={`rail-button ${active ? 'active' : ''}`}
      aria-label={label}
      title={label}
      onClick={onClick}
    >
      <Icon size={20} />
    </button>
  )
}

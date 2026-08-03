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
  TerminalSquare,
} from 'lucide-react'

export function TitleActions({
  context = false,
  onOpenSources,
  onOpenContext,
  onOpenFilters,
  onOpenHistory,
  onHistoryBack,
  onHistoryForward,
  canGoBack,
  canGoForward,
}: {
  context?: boolean
  onOpenSources?: () => void
  onOpenContext?: () => void
  onOpenFilters?: () => void
  onOpenHistory?: () => void
  onHistoryBack?: () => void
  onHistoryForward?: () => void
  canGoBack?: boolean
  canGoForward?: boolean
}) {
  if (context) {
    return (
      <div className="title-actions">
        <button
          type="button"
          aria-label="Filter documents"
          title="Filter documents"
          data-tooltip="Filter documents"
          className="quick-tooltip"
          onClick={onOpenFilters}
        >
          <Filter size={18} />
        </button>
        <button
          type="button"
          aria-label="Open conversations"
          title="Open conversations"
          data-tooltip="Open conversations"
          className="quick-tooltip"
          onClick={onOpenHistory}
        >
          <Clock3 size={18} />
        </button>
        <button
          type="button"
          className="mobile-button quick-tooltip"
          aria-label="Open agent context"
          title="Open agent context"
          data-tooltip="Open agent context"
          onClick={onOpenContext}
        >
          <PanelRightClose size={18} />
        </button>
      </div>
    )
  }

  return (
    <>
      <button
        type="button"
        className="mobile-button quick-tooltip"
        aria-label="Open sources"
        title="Open sources"
        data-tooltip="Open sources"
        onClick={onOpenSources}
      >
        <Menu size={19} />
      </button>
      <div className="history-buttons" role="group" aria-label="Search history">
        <button
          aria-label="Previous search query"
          title="Previous search query"
          data-tooltip="Previous search query"
          className="quick-tooltip"
          disabled={!canGoBack}
          onClick={onHistoryBack}
          type="button"
        >
          <ArrowLeft size={18} />
        </button>
        <button
          aria-label="Next search query"
          title="Next search query"
          data-tooltip="Next search query"
          className="quick-tooltip"
          disabled={!canGoForward}
          onClick={onHistoryForward}
          type="button"
        >
          <ArrowRight size={18} />
        </button>
      </div>
    </>
  )
}

export type AppView =
  'knowledge' | 'settings' | 'inbox' | 'conversations' | 'agent-tools' | 'index' | 'help'

export function Navigation({
  view,
  workspaceTab,
  onNavigate,
  onSearch,
  onOpenGraph,
  onOpenTimeline,
}: {
  view: AppView
  workspaceTab?: 'answer' | 'document' | 'sources' | 'graph' | 'timeline'
  onNavigate: (view: AppView) => void
  onSearch: () => void
  onOpenGraph: () => void
  onOpenTimeline: () => void
}) {
  return (
    <nav className="rail" aria-label="Primary">
      <div className="brand-mark" aria-label="Cortana">
        <CortanaBrandMark />
      </div>
      <RailButton icon={Search} label="Search" onClick={onSearch} />
      <RailButton
        icon={BookOpenText}
        label="Knowledge"
        active={view === 'knowledge'}
        onClick={() => onNavigate('knowledge')}
      />
      <RailButton
        icon={GitFork}
        label="Graph"
        active={view === 'knowledge' && workspaceTab === 'graph'}
        onClick={onOpenGraph}
      />
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
      <RailButton
        icon={CalendarDays}
        label="Timeline"
        active={view === 'knowledge' && workspaceTab === 'timeline'}
        onClick={onOpenTimeline}
      />
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
      type="button"
      className={`rail-button ${active ? 'active' : ''} quick-tooltip`}
      aria-label={label}
      title={label}
      data-tooltip={label}
      onClick={onClick}
    >
      <Icon size={20} />
    </button>
  )
}

function CortanaBrandMark() {
  return (
    <svg
      viewBox="0 0 1024 1024"
      fill="none"
      role="presentation"
      xmlns="http://www.w3.org/2000/svg"
      aria-hidden="true"
    >
      <rect x="88" y="88" width="848" height="848" rx="188" fill="#111d31" />
      <path
        d="M512 164C330 164 183 311 183 493s147 329 329 329 329-147 329-329S694 164 512 164Zm0 72c142 0 257 115 257 257S654 750 512 750 255 635 255 493 370 236 512 236Z"
        fill="#0d2748"
      />
      <path
        d="M482 266c-98 14-175 96-183 195l6 3c8-96 85-176 182-187zM542 266c98 11 175 93 183 194l-6 2c-8-97-86-176-184-187z"
        fill="#4ed5ff"
      />
      <path
        d="M510 334c-88 0-159 70-159 158s70 158 159 158 159-70 159-158-70-158-159-158Zm0 54c57 0 104 47 104 104s-47 104-104 104-104-47-104-104 47-104 104-104Z"
        fill="#5ad9ff"
      />
      <path
        d="M510 370c-36 0-66 28-66 64s30 64 66 64 66-28 66-64-30-64-66-64Zm0 25c23 0 41 18 41 39s-18 39-41 39-41-18-41-39 18-39 41-39Z"
        fill="#0e355f"
      />
    </svg>
  )
}

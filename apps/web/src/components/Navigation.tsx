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

import { Button } from './ui/Button'

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
        <Button
          variant="icon"
          aria-label="Filter documents"
          data-tooltip="Filter documents"
          className="quick-tooltip"
          onClick={onOpenFilters}
        >
          <Filter size={18} />
        </Button>
        <Button
          variant="icon"
          aria-label="Open conversations"
          data-tooltip="Open conversations"
          className="quick-tooltip"
          onClick={onOpenHistory}
        >
          <Clock3 size={18} />
        </Button>
        <Button
          variant="icon"
          className="mobile-button quick-tooltip"
          aria-label="Open agent context"
          data-tooltip="Open agent context"
          onClick={onOpenContext}
        >
          <PanelRightClose size={18} />
        </Button>
      </div>
    )
  }

  return (
    <>
      <Button
        variant="icon"
        className="mobile-button quick-tooltip"
        aria-label="Open sources"
        data-tooltip="Open sources"
        onClick={onOpenSources}
      >
        <Menu size={19} />
      </Button>
      <div className="history-buttons" role="group" aria-label="Search history">
        <Button
          variant="icon"
          aria-label="Previous search query"
          data-tooltip="Previous search query"
          className="quick-tooltip"
          disabled={!canGoBack}
          onClick={onHistoryBack}
          type="button"
        >
          <ArrowLeft size={18} />
        </Button>
        <Button
          variant="icon"
          aria-label="Next search query"
          data-tooltip="Next search query"
          className="quick-tooltip"
          disabled={!canGoForward}
          onClick={onHistoryForward}
          type="button"
        >
          <ArrowRight size={18} />
        </Button>
      </div>
    </>
  )
}
export type AppView =
  'knowledge' | 'settings' | 'inbox' | 'conversations' | 'agent-tools' | 'index' | 'help'

export function Navigation({
  view,
  workspaceTab,
  resultAvailable,
  onNavigate,
  onSearch,
  onOpenGraph,
  onOpenTimeline,
}: {
  view: AppView
  workspaceTab?: 'answer' | 'document' | 'sources' | 'graph' | 'timeline'
  resultAvailable: boolean
  onNavigate: (view: AppView) => void
  onSearch: () => void
  onOpenGraph: () => void
  onOpenTimeline: () => void
}) {
  const timelineDisabled = !resultAvailable
  return (
    <nav className="rail" aria-label="Primary">
      <div className="brand-mark" aria-label="Cortana">
        <img src="/app-icon.svg" alt="Cortana" className="brand-mark-icon" />
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
        disabled={timelineDisabled}
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
  disabled = false,
  onClick,
}: {
  icon: typeof Search
  label: string
  active?: boolean
  disabled?: boolean
  onClick: () => void
}) {
  const title = disabled ? `${label}: available once a search returns evidence` : label
  return (
    <Button
      variant="icon"
      className={`rail-button ${active ? 'active' : ''} quick-tooltip`}
      aria-label={label}
      data-tooltip={title}
      disabled={disabled}
      onClick={onClick}
    >
      <Icon size={20} />
    </Button>
  )
}

import {
  Fragment,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type FormEvent,
  type ReactNode,
  type RefObject,
} from 'react'
import {
  ArrowLeft,
  ArrowRight,
  BookOpenText,
  CircleHelp,
  Database,
  GitFork,
  Inbox,
  LoaderCircle,
  MessageCircle,
  Search,
  Settings,
  Sparkles,
  TerminalSquare,
} from 'lucide-react'

import type { AppView } from '@/components/Navigation'
import { Badge } from '@/components/shadcn/badge'
import { Button } from '@/components/shadcn/button'
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from '@/components/shadcn/breadcrumb'
import {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandShortcut,
} from '@/components/shadcn/command'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/shadcn/dropdown-menu'
import { Input } from '@/components/shadcn/input'
import { ScrollArea, ScrollBar } from '@/components/shadcn/scroll-area'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/shadcn/sheet'
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarProvider,
  SidebarSeparator,
  SidebarTrigger,
  useSidebar,
} from '@/components/shadcn/sidebar'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/shadcn/tooltip'
import { shortcutLabel } from '@/shortcuts'
import type { M7ActivityInbox } from '@/components/m7/M7ActivityInbox'
import { WorkspaceLogo } from '@/workspaceLogos'

type WorkspaceOption = { id: string; name: string; color: string | null }

export type M7NavigationProps = {
  view: AppView
  workspaceTab?: 'answer' | 'document' | 'sources' | 'graph' | 'timeline'
  onNavigate: (view: AppView) => void
  onOpenGraph: () => void
}

export type M7HeaderProps = {
  query: string
  loading: boolean
  searchRef: RefObject<HTMLInputElement | null>
  canGoBack: boolean
  canGoForward: boolean
  onQueryChange: (value: string) => void
  onSubmit: (event: FormEvent) => void
  onReflect: () => void
  onHistoryBack: () => void
  onHistoryForward: () => void
  onOpenSources: (origin?: HTMLElement | null) => void
  onOpenFilters: () => void
  onOpenHistory: () => void
  onOpenContext: (origin?: HTMLElement | null) => void
  onOpenCommands: (origin?: HTMLElement | null) => void
  workspaceName: string
  location: string
}

export type M7CommandPaletteProps = {
  open: boolean
  finalFocus: RefObject<HTMLElement | null>
  workspaces: WorkspaceOption[]
  onOpenChange: (open: boolean) => void
  onSearch: () => void
  onFilterDocuments: () => void
  onChooseWorkspace: (workspace: string) => void
  onOpenSettings: () => void
}

const navigationItems = [
  { view: 'knowledge' as const, label: 'Knowledge', icon: BookOpenText },
  { view: 'inbox' as const, label: 'Inbox', icon: Inbox },
  { view: 'conversations' as const, label: 'Conversations', icon: MessageCircle },
  { view: 'agent-tools' as const, label: 'Agent tools', icon: TerminalSquare },
  { view: 'index' as const, label: 'Index', icon: Database },
]

export function M7ApplicationHeader({
  query,
  loading,
  searchRef,
  canGoBack,
  canGoForward,
  onQueryChange,
  onSubmit,
  onReflect,
  onHistoryBack,
  onHistoryForward,
  onOpenSources,
  onOpenFilters,
  onOpenHistory,
  onOpenContext,
  onOpenCommands,
  workspaceName,
  location,
}: M7HeaderProps) {
  const actionsRef = useRef<HTMLButtonElement>(null)
  return (
    <header className="flex min-h-14 shrink-0 items-center gap-2 border-b bg-background/95 px-2 backdrop-blur md:px-4">
      <SidebarTrigger aria-label="Toggle navigation" />
      <div className="hidden items-center gap-1 sm:flex" role="group" aria-label="Search history">
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                variant="ghost"
                size="icon"
                aria-label="Previous search query"
                disabled={!canGoBack}
                onClick={onHistoryBack}
              />
            }
          >
            <ArrowLeft aria-hidden="true" />
          </TooltipTrigger>
          <TooltipContent>Previous search query</TooltipContent>
        </Tooltip>
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                variant="ghost"
                size="icon"
                aria-label="Next search query"
                disabled={!canGoForward}
                onClick={onHistoryForward}
              />
            }
          >
            <ArrowRight aria-hidden="true" />
          </TooltipTrigger>
          <TooltipContent>Next search query</TooltipContent>
        </Tooltip>
      </div>
      <Breadcrumb className="hidden min-w-0 lg:block">
        <BreadcrumbList>
          <BreadcrumbItem>{workspaceName}</BreadcrumbItem>
          <BreadcrumbSeparator />
          <BreadcrumbItem>
            <BreadcrumbPage>{location}</BreadcrumbPage>
          </BreadcrumbItem>
        </BreadcrumbList>
      </Breadcrumb>
      <form className="relative min-w-0 flex-1 lg:ml-auto lg:max-w-xl" onSubmit={onSubmit}>
        <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          ref={searchRef}
          aria-label="Search your knowledge"
          className="h-9 pr-16 pl-9"
          value={query}
          onChange={(event) => onQueryChange(event.target.value)}
        />
        <span className="pointer-events-none absolute top-1/2 right-2 -translate-y-1/2 text-xs text-muted-foreground">
          {loading ? (
            <LoaderCircle className="size-4 animate-spin" aria-label="Searching" />
          ) : (
            <kbd>{shortcutLabel('MOD K')}</kbd>
          )}
        </span>
      </form>
      <Button
        type="button"
        size="sm"
        aria-label="Reflect on this objective"
        onClick={onReflect}
        disabled={loading || !query.trim()}
      >
        <Sparkles aria-hidden="true" />
        <span className="hidden md:inline">Reflect</span>
      </Button>
      <DropdownMenu>
        <DropdownMenuTrigger render={<Button ref={actionsRef} variant="outline" size="sm" />}>
          Actions
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuGroup>
            <DropdownMenuLabel>Workspace</DropdownMenuLabel>
            <DropdownMenuItem onClick={() => onOpenSources(actionsRef.current)}>
              Open sources
            </DropdownMenuItem>
            <DropdownMenuItem onClick={onOpenFilters}>Filter documents</DropdownMenuItem>
            <DropdownMenuItem onClick={() => onOpenContext(actionsRef.current)}>
              Open agent context
            </DropdownMenuItem>
            <DropdownMenuItem onClick={onOpenHistory}>Open conversations</DropdownMenuItem>
          </DropdownMenuGroup>
          <DropdownMenuSeparator />
          <DropdownMenuGroup>
            <DropdownMenuLabel>Search history</DropdownMenuLabel>
            <DropdownMenuItem disabled={!canGoBack} onClick={onHistoryBack}>
              Previous query
            </DropdownMenuItem>
            <DropdownMenuItem disabled={!canGoForward} onClick={onHistoryForward}>
              Next query
            </DropdownMenuItem>
            <DropdownMenuItem onClick={() => onOpenCommands(actionsRef.current)}>
              Command palette
              <span className="ml-auto text-xs text-muted-foreground">
                {shortcutLabel('MOD P')}
              </span>
            </DropdownMenuItem>
          </DropdownMenuGroup>
        </DropdownMenuContent>
      </DropdownMenu>
    </header>
  )
}

export function M7CommandPalette({
  open,
  finalFocus,
  workspaces,
  onOpenChange,
  onSearch,
  onFilterDocuments,
  onChooseWorkspace,
  onOpenSettings,
}: M7CommandPaletteProps) {
  const run = (action: () => void) => {
    onOpenChange(false)
    action()
  }

  return (
    <CommandDialog
      open={open}
      onOpenChange={onOpenChange}
      finalFocus={finalFocus}
      title="Cortana command palette"
      description="Search navigation and workspace commands"
    >
      <Command label="Search Cortana commands">
        <CommandInput placeholder="Search commands…" />
        <CommandList>
          <CommandEmpty>No commands found.</CommandEmpty>
          <CommandGroup heading="Actions">
            <CommandItem onSelect={() => run(onSearch)}>
              Search the brain
              <CommandShortcut>{shortcutLabel('MOD K')}</CommandShortcut>
            </CommandItem>
            <CommandItem onSelect={() => run(onFilterDocuments)}>
              Filter documents
              <CommandShortcut>{shortcutLabel('MOD ⇧ F')}</CommandShortcut>
            </CommandItem>
            <CommandItem onSelect={() => run(onOpenSettings)}>Open settings</CommandItem>
          </CommandGroup>
          <CommandGroup heading="Workspaces">
            {workspaces.map((item) => (
              <CommandItem key={item.id} onSelect={() => run(() => onChooseWorkspace(item.id))}>
                Switch to {item.name}
              </CommandItem>
            ))}
          </CommandGroup>
        </CommandList>
      </Command>
    </CommandDialog>
  )
}

export type M7StatusBarProps = { children: ReactNode; demo: boolean }

export type M7PanelBoundaryProps = {
  enabled: boolean
  side: 'left' | 'right'
  breakpoint: number
  open: boolean
  title: string
  description: string
  finalFocus: RefObject<HTMLElement | null>
  onOpenChange: (open: boolean) => void
  children: ReactNode
}

export function M7PanelBoundary({
  enabled,
  side,
  breakpoint,
  open,
  title,
  description,
  finalFocus,
  onOpenChange,
  children,
}: M7PanelBoundaryProps) {
  const [compact, setCompact] = useState(false)

  useEffect(() => {
    if (!enabled) return
    const query = window.matchMedia(`(max-width: ${breakpoint - 1}px)`)
    const update = () => setCompact(query.matches)
    update()
    query.addEventListener('change', update)
    return () => query.removeEventListener('change', update)
  }, [breakpoint, enabled])

  if (!enabled || !compact) return children
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        finalFocus={finalFocus}
        side={side}
        className="m7-panel-boundary w-[min(390px,92vw)] max-w-none gap-0 p-0"
      >
        <SheetHeader className="sr-only">
          <SheetTitle>{title}</SheetTitle>
          <SheetDescription>{description}</SheetDescription>
        </SheetHeader>
        {children}
      </SheetContent>
    </Sheet>
  )
}

export function M7StatusBar({ children, demo }: M7StatusBarProps) {
  return (
    <footer className="shrink-0 border-t bg-background" aria-label="Application status">
      <ScrollArea className="w-full whitespace-nowrap">
        <div className="flex min-h-10 items-center gap-3 px-3 py-1.5 text-xs text-muted-foreground">
          {children}
          <span className="ml-auto" />
          {demo ? <Badge variant="secondary">Demo data</Badge> : null}
        </div>
        <ScrollBar orientation="horizontal" />
      </ScrollArea>
    </footer>
  )
}

export function M7ApplicationNavigation({
  navigation,
  workspaces,
  workspace,
  onWorkspaceChange,
}: {
  navigation: M7NavigationProps
  workspaces: WorkspaceOption[]
  workspace: string
  onWorkspaceChange: (workspace: string) => void
}) {
  const activeWorkspace = workspaces.find((item) => item.id === workspace)
  const { isMobile, mobileFinalFocusRef, mobileTriggerRef, setOpenMobile } = useSidebar()
  const runNavigation = (action: () => void, focusDestination = false) => {
    action()
    if (isMobile) {
      mobileFinalFocusRef.current =
        focusDestination && document.activeElement instanceof HTMLElement
          ? document.activeElement
          : mobileTriggerRef.current
      setOpenMobile(false)
    }
  }

  return (
    <Sidebar
      variant="sidebar"
      collapsible="icon"
      role="navigation"
      aria-label="Primary navigation"
      className="m7-application-sidebar"
    >
      <SidebarHeader className="p-3">
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <SidebarMenuButton
                size="lg"
                className="h-12 p-0 group-data-[collapsible=icon]:size-12!"
                tooltip={`Workspace: ${activeWorkspace?.name ?? 'Choose workspace'}`}
                aria-label="Switch workspace"
              />
            }
          >
            {activeWorkspace ? (
              <WorkspaceLogo workspace={activeWorkspace} size="large" />
            ) : (
              <span
                className="workspace-logo workspace-logo--large workspace-picker-mark"
                aria-hidden="true"
              >
                ?
              </span>
            )}
            <span
              data-workspace-labels
              className="min-w-0 flex-1 pr-2 text-left group-data-[collapsible=icon]:hidden"
            >
              <span className="block truncate text-sm font-medium">
                {activeWorkspace?.name ?? 'Choose workspace'}
              </span>
              <span className="block truncate text-xs text-muted-foreground">Workspace</span>
            </span>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" sideOffset={6} className="min-w-56">
            <DropdownMenuGroup>
              <DropdownMenuLabel>Workspaces</DropdownMenuLabel>
              <DropdownMenuRadioGroup
                value={workspace}
                onValueChange={(value) => runNavigation(() => onWorkspaceChange(value))}
              >
                {workspaces.map((item) => (
                  <DropdownMenuRadioItem key={item.id} value={item.id}>
                    <WorkspaceLogo workspace={item} size="small" />
                    {item.name}
                  </DropdownMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
            </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>
      </SidebarHeader>
      <SidebarSeparator />
      <SidebarContent>
        <SidebarGroup className="p-3">
          <SidebarGroupLabel>Workspace</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu className="gap-0.5">
              {navigationItems.map(({ view, label, icon: Icon }) => (
                <Fragment key={view}>
                  <SidebarMenuItem>
                    <SidebarMenuButton
                      size="lg"
                      tooltip={label}
                      isActive={
                        navigation.view === view &&
                        (view !== 'knowledge' || navigation.workspaceTab !== 'graph')
                      }
                      aria-current={
                        navigation.view === view &&
                        (view !== 'knowledge' || navigation.workspaceTab !== 'graph')
                          ? 'page'
                          : undefined
                      }
                      onClick={() => runNavigation(() => navigation.onNavigate(view))}
                    >
                      <Icon aria-hidden="true" />
                      <span>{label}</span>
                    </SidebarMenuButton>
                  </SidebarMenuItem>
                  {view === 'knowledge' && (
                    <SidebarMenuItem>
                      <SidebarMenuButton
                        size="lg"
                        tooltip="Graph"
                        isActive={
                          navigation.view === 'knowledge' && navigation.workspaceTab === 'graph'
                        }
                        aria-current={
                          navigation.view === 'knowledge' && navigation.workspaceTab === 'graph'
                            ? 'page'
                            : undefined
                        }
                        onClick={() => runNavigation(navigation.onOpenGraph)}
                      >
                        <GitFork aria-hidden="true" />
                        <span>Graph</span>
                      </SidebarMenuButton>
                    </SidebarMenuItem>
                  )}
                </Fragment>
              ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>
      <SidebarFooter className="p-3">
        <SidebarMenu className="gap-0.5">
          <SidebarMenuItem>
            <SidebarMenuButton
              size="lg"
              tooltip="Settings"
              isActive={navigation.view === 'settings'}
              aria-current={navigation.view === 'settings' ? 'page' : undefined}
              onClick={() => runNavigation(() => navigation.onNavigate('settings'))}
            >
              <Settings aria-hidden="true" />
              <span>Settings</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
          <SidebarMenuItem>
            <SidebarMenuButton
              size="lg"
              tooltip="Help"
              isActive={navigation.view === 'help'}
              aria-current={navigation.view === 'help' ? 'page' : undefined}
              onClick={() => runNavigation(() => navigation.onNavigate('help'))}
            >
              <CircleHelp aria-hidden="true" />
              <span>Help</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarFooter>
    </Sidebar>
  )
}

export function M7ShellProvider({ enabled, children }: { enabled: boolean; children: ReactNode }) {
  if (!enabled) return children
  return (
    <TooltipProvider delay={250}>
      <SidebarProvider
        defaultOpen={false}
        className="m7-shell-provider min-h-0 overflow-hidden"
        style={{ '--sidebar-width-icon': '4.5rem' } as CSSProperties}
      >
        {children}
      </SidebarProvider>
    </TooltipProvider>
  )
}

export type M7ShellProviderProps = { enabled: boolean; children: ReactNode }

export type M7ShellComponents = {
  ActivityInbox: typeof M7ActivityInbox
  ApplicationHeader: typeof M7ApplicationHeader
  ApplicationNavigation: typeof M7ApplicationNavigation
  CommandPalette: typeof M7CommandPalette
  PanelBoundary: typeof M7PanelBoundary
  ShellProvider: typeof M7ShellProvider
  StatusBar: typeof M7StatusBar
}

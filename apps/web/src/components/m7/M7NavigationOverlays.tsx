import { useEffect, useRef, useState } from 'react'
import {
  BookOpenText,
  Database,
  MoreHorizontal,
  Search,
  Settings,
  ShieldCheck,
  SlidersHorizontal,
  TextSearch,
  Workflow,
} from 'lucide-react'

import { Badge } from '@/components/shadcn/badge'
import { Button } from '@/components/shadcn/button'
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/shadcn/card'
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
  ContextMenu,
  ContextMenuContent,
  ContextMenuGroup,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from '@/components/shadcn/context-menu'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/shadcn/dropdown-menu'
import { Field, FieldDescription, FieldLabel } from '@/components/shadcn/field'
import {
  Pagination,
  PaginationContent,
  PaginationItem,
  PaginationLink,
  PaginationNext,
  PaginationPrevious,
} from '@/components/shadcn/pagination'
import {
  Popover,
  PopoverContent,
  PopoverDescription,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
} from '@/components/shadcn/popover'
import { Switch } from '@/components/shadcn/switch'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/shadcn/tooltip'

const workspaces = ['Personal', 'Product', 'Research']

export function WorkspaceSwitcher({
  workspace,
  onWorkspaceChange,
}: {
  workspace: string
  onWorkspaceChange: (workspace: string) => void
}) {
  return (
    <div className="contents" data-m7-workspace-overlays-ready>
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              variant="ghost"
              className="h-10 w-full justify-start gap-2 px-2"
              aria-label={`Switch workspace. Current workspace: ${workspace}`}
            />
          }
        >
          <img className="size-7 rounded-lg" src="/app-icon.svg" alt="" />
          <div className="min-w-0 text-left group-data-[collapsible=icon]:hidden">
            <p className="font-heading truncate text-sm font-medium">{workspace}</p>
            <p className="truncate text-xs text-muted-foreground">Evidence workbench</p>
          </div>
        </DropdownMenuTrigger>
        <DropdownMenuContent aria-label="Workspaces" sideOffset={6}>
          <DropdownMenuGroup>
            <DropdownMenuLabel>Workspaces</DropdownMenuLabel>
            {workspaces.map((item) => (
              <DropdownMenuItem key={item} onClick={() => onWorkspaceChange(item)}>
                {item}
              </DropdownMenuItem>
            ))}
          </DropdownMenuGroup>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  )
}

export function HeaderOverlayActions({
  onWorkspaceChange,
}: {
  onWorkspaceChange: (workspace: string) => void
}) {
  const [commandOpen, setCommandOpen] = useState(false)
  const commandTriggerRef = useRef<HTMLButtonElement>(null)
  const commandReturnFocusRef = useRef<HTMLElement | null>(null)

  useEffect(() => {
    const openCommand = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
        event.preventDefault()
        commandReturnFocusRef.current =
          document.activeElement instanceof HTMLElement ? document.activeElement : null
        setCommandOpen(true)
      }
    }
    window.addEventListener('keydown', openCommand)
    return () => window.removeEventListener('keydown', openCommand)
  }, [])

  const closeCommand = () => {
    setCommandOpen(false)
    const returnTarget = commandReturnFocusRef.current ?? commandTriggerRef.current
    commandReturnFocusRef.current = null
    requestAnimationFrame(() => returnTarget?.focus())
  }

  return (
    <div className="contents" data-m7-header-overlays-ready>
      <Popover>
        <PopoverTrigger
          render={<Button variant="ghost" size="icon-sm" aria-label="Filter evidence" />}
        >
          <SlidersHorizontal />
        </PopoverTrigger>
        <PopoverContent align="end" aria-label="Evidence filters">
          <PopoverHeader>
            <PopoverTitle>Evidence filters</PopoverTitle>
            <PopoverDescription>
              Narrow the visible evidence without expanding retrieval scope.
            </PopoverDescription>
          </PopoverHeader>
          <Field orientation="horizontal">
            <div>
              <FieldLabel htmlFor="prototype-recent-only">Recent sources only</FieldLabel>
              <FieldDescription>Updated in the last 30 days.</FieldDescription>
            </div>
            <Switch id="prototype-recent-only" />
          </Field>
        </PopoverContent>
      </Popover>
      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              ref={commandTriggerRef}
              variant="ghost"
              size="icon-sm"
              aria-label="Open command palette"
              onClick={() => {
                commandReturnFocusRef.current = commandTriggerRef.current
                setCommandOpen(true)
              }}
            />
          }
        >
          <Workflow />
        </TooltipTrigger>
        <TooltipContent>
          Commands <kbd className="rounded bg-background/20 px-1">⌘K</kbd>
        </TooltipContent>
      </Tooltip>
      <CommandDialog
        open={commandOpen}
        onOpenChange={(open) => (open ? setCommandOpen(true) : closeCommand())}
        title="Command palette"
        description="Navigate Cortana without changing retrieval scope."
      >
        <Command label="Search commands">
          <CommandInput placeholder="Search commands…" />
          <CommandList>
            <CommandEmpty>No commands found.</CommandEmpty>
            <CommandGroup heading="Navigate">
              <CommandItem onSelect={closeCommand}>
                <Search /> Search the brain
                <CommandShortcut>⌘K</CommandShortcut>
              </CommandItem>
              <CommandItem onSelect={closeCommand}>
                <Settings /> Open settings
              </CommandItem>
            </CommandGroup>
            <CommandGroup heading="Workspace">
              {workspaces.map((item) => (
                <CommandItem
                  key={item}
                  onSelect={() => {
                    onWorkspaceChange(item)
                    closeCommand()
                  }}
                >
                  <Database /> Switch to {item}
                </CommandItem>
              ))}
            </CommandGroup>
          </CommandList>
        </Command>
      </CommandDialog>
    </div>
  )
}

export function EvidenceDocumentCard({ onReview }: { onReview: () => void }) {
  const [contextNotice, setContextNotice] = useState('')

  return (
    <div data-m7-evidence-overlays-ready>
      <ContextMenu>
        <ContextMenuTrigger render={<Card />}>
          <CardHeader>
            <CardTitle>How do releases work?</CardTitle>
            <CardDescription>work-drive / release-process · updated Jul 28</CardDescription>
            <CardAction>
              <div className="flex items-center gap-1">
                <Badge variant="secondary">98% match</Badge>
                <DropdownMenu>
                  <DropdownMenuTrigger
                    render={
                      <Button variant="ghost" size="icon-xs" aria-label="Open evidence actions" />
                    }
                  >
                    <MoreHorizontal />
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end" aria-label="Evidence actions">
                    <DropdownMenuGroup>
                      <DropdownMenuLabel>Evidence actions</DropdownMenuLabel>
                      <DropdownMenuItem onClick={() => setContextNotice('Opened source')}>
                        <BookOpenText /> Open source
                      </DropdownMenuItem>
                      <DropdownMenuItem onClick={() => setContextNotice('Copied citation')}>
                        <TextSearch /> Copy citation
                      </DropdownMenuItem>
                    </DropdownMenuGroup>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem onClick={onReview}>
                      <ShieldCheck /> Review context boundary
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              </div>
            </CardAction>
          </CardHeader>
          <CardContent className="flex flex-col gap-4 leading-6">
            <p>
              Releases follow trunk-based development with short-lived feature branches and
              automated delivery from main.
            </p>
            <div className="border-l-2 border-primary pl-4">
              <p className="font-medium text-foreground">Evidence spine</p>
              <p className="text-muted-foreground">
                Plan against the roadmap, validate the pull request, then cut and monitor the
                release. Roll back if health checks regress.
              </p>
            </div>
          </CardContent>
        </ContextMenuTrigger>
        <ContextMenuContent aria-label="Evidence actions">
          <ContextMenuGroup>
            <ContextMenuLabel>Evidence actions</ContextMenuLabel>
            <ContextMenuItem onClick={() => setContextNotice('Opened source')}>
              <BookOpenText /> Open source
            </ContextMenuItem>
            <ContextMenuItem onClick={() => setContextNotice('Copied citation')}>
              <TextSearch /> Copy citation
            </ContextMenuItem>
          </ContextMenuGroup>
          <ContextMenuSeparator />
          <ContextMenuItem onClick={onReview}>
            <ShieldCheck /> Review context boundary
          </ContextMenuItem>
        </ContextMenuContent>
        <div className="mt-3 flex flex-wrap items-center justify-between gap-2">
          <span className="text-xs text-muted-foreground" role="status">
            {contextNotice}
          </span>
          <Pagination className="mx-0 w-auto" aria-label="Evidence pages">
            <PaginationContent>
              <PaginationItem>
                <PaginationPrevious href="#previous" />
              </PaginationItem>
              <PaginationItem>
                <PaginationLink href="#page-1" isActive>
                  1
                </PaginationLink>
              </PaginationItem>
              <PaginationItem>
                <PaginationLink href="#page-2">2</PaginationLink>
              </PaginationItem>
              <PaginationItem>
                <PaginationNext href="#next" />
              </PaginationItem>
            </PaginationContent>
          </Pagination>
        </div>
      </ContextMenu>
    </div>
  )
}

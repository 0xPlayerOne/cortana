import { useState } from 'react'
import {
  BookOpenText,
  Braces,
  Database,
  FileSearch,
  Network,
  Search,
  Settings,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
} from 'lucide-react'

import { Alert, AlertDescription, AlertTitle } from '@/components/shadcn/alert'
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
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/shadcn/dialog'
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@/components/shadcn/field'
import { Input } from '@/components/shadcn/input'
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarInset,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarProvider,
  SidebarSeparator,
  SidebarTrigger,
} from '@/components/shadcn/sidebar'
import { Switch } from '@/components/shadcn/switch'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/shadcn/tabs'
import { TooltipProvider } from '@/components/shadcn/tooltip'
import '@/shadcn.css'

const navigation = [
  { label: 'Knowledge', icon: BookOpenText, active: true },
  { label: 'Search', icon: FileSearch },
  { label: 'Graph', icon: Network },
  { label: 'Memory', icon: Database, count: '42' },
  { label: 'Agent tools', icon: Braces },
]

export function M7ShadcnPrototype() {
  const [dialogOpen, setDialogOpen] = useState(false)

  return (
    <TooltipProvider delay={250}>
      <SidebarProvider defaultOpen>
        <Sidebar variant="inset" collapsible="icon">
          <SidebarHeader>
            <div className="flex h-10 items-center gap-2 px-2">
              <img className="size-7 rounded-lg" src="/app-icon.svg" alt="" />
              <div className="min-w-0 group-data-[collapsible=icon]:hidden">
                <p className="font-heading text-sm font-medium">Cortana</p>
                <p className="truncate text-xs text-muted-foreground">Evidence workbench</p>
              </div>
            </div>
          </SidebarHeader>
          <SidebarSeparator />
          <SidebarContent>
            <SidebarGroup>
              <SidebarGroupLabel>Workspace</SidebarGroupLabel>
              <SidebarGroupContent>
                <SidebarMenu>
                  {navigation.map(({ label, icon: Icon, active, count }) => (
                    <SidebarMenuItem key={label}>
                      <SidebarMenuButton isActive={active} tooltip={label}>
                        <Icon data-icon="inline-start" />
                        <span>{label}</span>
                      </SidebarMenuButton>
                      {count ? <SidebarMenuBadge>{count}</SidebarMenuBadge> : null}
                    </SidebarMenuItem>
                  ))}
                </SidebarMenu>
              </SidebarGroupContent>
            </SidebarGroup>
            <SidebarGroup>
              <SidebarGroupLabel>Sources</SidebarGroupLabel>
              <SidebarGroupContent className="flex flex-col gap-1 px-2 text-xs text-sidebar-foreground/80">
                <p className="flex items-center justify-between gap-2">
                  Work Drive <span className="tabular-nums">1,982</span>
                </p>
                <p className="flex items-center justify-between gap-2">
                  Team Slack <span className="tabular-nums">623</span>
                </p>
              </SidebarGroupContent>
            </SidebarGroup>
          </SidebarContent>
          <SidebarFooter>
            <SidebarMenu>
              <SidebarMenuItem>
                <SidebarMenuButton tooltip="Settings">
                  <Settings data-icon="inline-start" />
                  <span>Settings</span>
                </SidebarMenuButton>
              </SidebarMenuItem>
            </SidebarMenu>
          </SidebarFooter>
        </Sidebar>

        <SidebarInset className="min-h-svh overflow-hidden">
          <header className="flex h-14 shrink-0 items-center gap-2 border-b px-3 md:px-4">
            <SidebarTrigger aria-label="Toggle navigation" />
            <div className="relative min-w-0 flex-1 md:max-w-2xl">
              <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
              <Input
                aria-label="Search your knowledge"
                className="h-9 pl-9"
                defaultValue="How do releases work?"
              />
            </div>
            <Button size="sm" aria-label="Reflect on this objective">
              <Sparkles data-icon="inline-start" />
              <span className="hidden sm:inline">Reflect</span>
            </Button>
            <Button variant="ghost" size="icon-sm" aria-label="Filter evidence">
              <SlidersHorizontal />
            </Button>
          </header>

          <main className="min-h-0 flex-1 overflow-auto p-3 md:p-5" id="main-content">
            <div className="mx-auto grid max-w-7xl gap-4 xl:grid-cols-[minmax(0,1fr)_20rem]">
              <section className="flex min-w-0 flex-col gap-4" aria-labelledby="evidence-title">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div>
                    <p className="text-xs font-medium tracking-[0.16em] text-primary uppercase">
                      Personal / All sources
                    </p>
                    <h1 id="evidence-title" className="font-heading text-2xl font-medium">
                      Release evidence
                    </h1>
                  </div>
                  <div className="flex items-center gap-2">
                    <Badge variant="outline">4 sources</Badge>
                    <Badge>Index online</Badge>
                  </div>
                </div>

                <Alert>
                  <ShieldCheck />
                  <AlertTitle>Bounded, provenance-first context</AlertTitle>
                  <AlertDescription>
                    Every answer stays linked to its source and workspace policy.
                  </AlertDescription>
                </Alert>

                <Tabs defaultValue="document">
                  <TabsList variant="line" aria-label="Evidence view">
                    <TabsTrigger value="document">Document</TabsTrigger>
                    <TabsTrigger value="answer">Answer</TabsTrigger>
                    <TabsTrigger value="context">Context</TabsTrigger>
                  </TabsList>
                  <TabsContent value="document" className="pt-3">
                    <Card>
                      <CardHeader>
                        <CardTitle>How do releases work?</CardTitle>
                        <CardDescription>
                          work-drive / release-process · updated Jul 28
                        </CardDescription>
                        <CardAction>
                          <Badge variant="secondary">98% match</Badge>
                        </CardAction>
                      </CardHeader>
                      <CardContent className="flex flex-col gap-4 leading-6">
                        <p>
                          Releases follow trunk-based development with short-lived feature branches
                          and automated delivery from main.
                        </p>
                        <div className="border-l-2 border-primary pl-4">
                          <p className="font-medium text-foreground">Evidence spine</p>
                          <p className="text-muted-foreground">
                            Plan against the roadmap, validate the pull request, then cut and
                            monitor the release. Roll back if health checks regress.
                          </p>
                        </div>
                      </CardContent>
                    </Card>
                  </TabsContent>
                  <TabsContent value="answer" className="pt-3 text-muted-foreground">
                    Synthesized answers will compose the same evidence cards and citations.
                  </TabsContent>
                  <TabsContent value="context" className="pt-3 text-muted-foreground">
                    Agent context will show token budgets, included sources, and policy boundaries.
                  </TabsContent>
                </Tabs>
              </section>

              <aside className="flex flex-col gap-4" aria-labelledby="prototype-settings-title">
                <Card size="sm">
                  <CardHeader>
                    <CardTitle id="prototype-settings-title">Retrieval settings</CardTitle>
                    <CardDescription>Representative M7 form composition</CardDescription>
                  </CardHeader>
                  <CardContent>
                    <FieldGroup>
                      <Field>
                        <FieldLabel htmlFor="prototype-workspace">Workspace</FieldLabel>
                        <Input id="prototype-workspace" defaultValue="Personal" />
                        <FieldDescription>
                          Scopes documents, memory, and agent context.
                        </FieldDescription>
                      </Field>
                      <Field orientation="horizontal">
                        <div>
                          <FieldLabel htmlFor="prototype-citations">Require citations</FieldLabel>
                          <FieldDescription>Reject unsupported synthesis.</FieldDescription>
                        </div>
                        <Switch id="prototype-citations" defaultChecked />
                      </Field>
                    </FieldGroup>
                  </CardContent>
                </Card>
                <Button variant="outline" className="w-full" onClick={() => setDialogOpen(true)}>
                  Review context boundary
                </Button>
              </aside>
            </div>
          </main>

          <footer className="flex min-h-9 flex-wrap items-center gap-x-4 gap-y-1 border-t px-3 py-2 text-xs text-muted-foreground">
            <span className="text-foreground">Index online</span>
            <span>9,834 documents</span>
            <span>Query: synthesized</span>
            <span className="ml-auto text-primary">M7 preview</span>
          </footer>
        </SidebarInset>

        <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Agent context boundary</DialogTitle>
              <DialogDescription>
                This preview contains four citations from the Personal workspace and no credentials.
              </DialogDescription>
            </DialogHeader>
            <div className="rounded-lg border bg-muted p-3 text-sm">
              1,642 estimated tokens · four evidence records · one active workspace
            </div>
            <DialogFooter>
              <Button variant="outline" onClick={() => setDialogOpen(false)}>
                Cancel
              </Button>
              <Button onClick={() => setDialogOpen(false)}>Copy bounded context</Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </SidebarProvider>
    </TooltipProvider>
  )
}

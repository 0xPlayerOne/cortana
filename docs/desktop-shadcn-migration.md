# Desktop shadcn migration record

This record owns the reproducible baseline, architecture decision, legacy inventory, and issue
sequence for M7. The product acceptance contract remains in
[`desktop-ux-audit.md`](desktop-ux-audit.md); this file records the migration evidence needed to
apply it.

## Locked foundation

| Decision            | M7 contract                                                                                                                                                                    |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Registry            | Official `@shadcn` registry only                                                                                                                                               |
| Component base      | Base UI                                                                                                                                                                        |
| Style               | Nova (`base-nova` in `components.json`)                                                                                                                                        |
| Icons               | Lucide; decorative icons are hidden and action icons retain accessible names                                                                                                   |
| Font                | Geist Variable for the renderer heading and sans contracts                                                                                                                     |
| Styling             | Tailwind CSS 4 with CSS variables and semantic Cortana tokens                                                                                                                  |
| Generated UI alias  | `@/components/shadcn`                                                                                                                                                          |
| Utilities and hooks | `@/lib`, `@/lib/utils`, and `@/hooks`                                                                                                                                          |
| Renderer transition | Legacy is the packaged default until M7 cleanup; `VITE_CORTANA_RENDERER=shadcn` enables the migration renderer, and `?renderer=shadcn` is accepted only by a development build |

The generated directory is deliberately separate from the temporary `components/ui/Button.tsx`
legacy owner. This prevents registry updates from overwriting the renderer that remains packaged
during the transition. Issue #2167 removes that legacy directory and the flag after all callers
have migrated.

All four Cortana themes map the same semantic variables. `background`, `foreground`, `card`,
`popover`, `primary`, `secondary`, `muted`, `accent`, `destructive`, `border`, `input`, `ring`,
chart, and sidebar tokens resolve through the existing theme palette. Theme selection changes
values only; component structure and interaction do not fork.

## Reproducible visual baseline

The fixture is the non-secret `?demo=1` dataset in `apps/web/src/demo.ts`. It uses `example.test`
URLs, invented people and channels, and no local paths or credentials.

From a checkout with dependencies installed:

```sh
bun run dev -- --host 127.0.0.1 --port 4173
bun scripts/capture-m7-visuals.mjs \
  --renderer legacy \
  --base-url http://127.0.0.1:4173 \
  --output artifacts/m7-shadcn/before
VITE_CORTANA_RENDERER=shadcn bun run build
bun run --cwd apps/web preview -- --host 127.0.0.1 --port 4174
bun scripts/capture-m7-visuals.mjs \
  --renderer shadcn \
  --base-url http://127.0.0.1:4174 \
  --output artifacts/m7-shadcn/production-shell
```

The preview command is the production-shell evidence source and bundles Geist locally. The pull
request workflow may use Vite development mode because its clean Linux install resolves font assets
inside the checkout; local Bun cache symlinks on macOS can otherwise produce a dev-only font 403 that
does not exist in the production build.

Install the matching browser once with `bunx playwright install chromium` when Playwright reports
that it is missing. The capture fails on browser console errors. The legacy run records 56 images:
all primary wide-screen destinations; the default and accessibility themes at 320, 768, 1024,
and 1440 CSS pixels; Forest and Plum at compact and desktop widths; command, source-sheet,
settings, and graph states. The production-shell run records 31 images: the real data-backed shell
at all four widths in all four themes, mobile navigation, tablet source/context panels, command and
workspace overlays, Inbox, Settings, and populated answer, conversations, evidence, timeline,
canonical-document, and bounded-graph views, with populated conversations at mobile and desktop
widths. Exceptional native settings states remain part of #2166 rather than being
represented by the retired static prototype.

`.github/workflows/m7-visual-evidence.yml` runs the same capture on the exact pull-request revision,
audits every production-shell theme/width against WCAG 2.2 AA automation, and uploads the complete
non-secret matrix as a 30-day GitHub Actions artifact. Link the exact run from the issue or pull
request; local artifact paths alone are not acceptance evidence. Final packaged evidence uses the
longer-lived release record required by the Desktop UX audit.

The automated interaction gate also verifies keyboard-opened mobile navigation and focus return,
the command palette, workspace and action menus, source-panel reachability, reduced-motion Sheet
behavior, and a 720-CSS-pixel layout at 2x density as the reflow equivalent of a
1,440-physical-pixel window at 200% zoom. Issue #2168 retains the real packaged 200% zoom and
assistive-technology checks.

The baseline exposed acceptance failures that M7 must not preserve:

- the primary navigation is absent below 781 CSS pixels, leaving Settings, Help, Inbox, Graph,
  Memory, and Agent tools unreachable;
- the 320-pixel title bar clips the Reflect action and the status bar overflows horizontally;
- the web fixture cannot render populated Desktop settings, setup, service, recovery, backup, or
  update states, so M7 must add deterministic typed native-bridge fixtures before final evidence;
- the command surface manually implements modal semantics and focus instead of using the shared
  overlay primitives;
- similarly named Conversations actions make broad accessible-name selectors ambiguous;
- custom breakpoint and selector families duplicate component behavior across files.

## Baseline measurements

Measurements were captured on the deterministic Vite development fixture on 2026-08-26. They are
diagnostic comparison points, not packaged performance claims.

| Measurement                          |  Legacy baseline | Foundation prototype |
| ------------------------------------ | ---------------: | -------------------: |
| Median DOM content loaded, five runs |           340 ms |               323 ms |
| Median load event, five runs         |           342 ms |               325 ms |
| Median network-idle ready, five runs |           961 ms |             1,069 ms |
| Median Chromium JS heap used         | 16,973,928 bytes |     19,736,116 bytes |
| DOM nodes                            |              535 |                  328 |
| Layout count                         |                4 |                    5 |

The pre-migration production build transformed 1,699 modules in 1.55 seconds. Its renderer entry
was 453,729 bytes (133.57 kB gzip), and legacy CSS was 72,299 bytes (13.67 kB gzip). The flag-disabled
foundation build keeps the shadcn prototype in separate lazy chunks: the complete legacy-default
initial graph is 455.07 kB, while the prototype is 206.05 kB (66.58 kB gzip) plus the complete
shared primitive CSS contract at 121.35 kB (18.83 kB gzip). Issue #2167 must compare the final
single renderer to the baseline and remove transition-only
code before issue #2168 can accept performance.

`bun run build` enforces reviewed uncompressed budgets from the Vite manifest: 475,000 bytes for
the complete legacy-default initial JavaScript graph, 80,000 bytes for its CSS, 50,000 bytes for
the lazy production-shell entry, 650,000 bytes for its complete incremental static import graph
beyond the mode resolver, and 210,000 bytes for its complete CSS graph. The earlier #2163 overlay
slice measured 40.55 kB for the prototype renderer entry and 322.59 kB for its complete incremental
graph; recursive graph measurement continues to prevent manual chunking from hiding imported
JavaScript. The first real production-shell build replaces the static
prototype entry while preserving the legacy default. After the #2165 knowledge-workspace slice, the
legacy default is 468.449 kB JavaScript and 72.299 kB CSS. Its 1.149 kB lazy entry reaches a
621.839 kB incremental static graph containing the complete real application plus the injected
shadcn surface primitives, and its combined legacy-content plus semantic-shell CSS graph is
206.680 kB. The reviewed transition caps are therefore 650,000 bytes for that production graph and
210,000 bytes for its CSS; #2165 through #2167 must reduce those numbers as legacy surfaces and
styles leave the graph. The final migration replaces all transition budgets rather than silently
carrying them forward.

The added JavaScript packages use MIT or Apache-2.0 licenses. The bundled Geist font package uses
OFL-1.1. No copyleft runtime, native library, hosted font request, or new executable is introduced.
`bun audit --production` reports no known vulnerabilities after pinning the remediated transitive
`js-yaml` and `nanoid` releases in `bun.lock`.

## Shared primitive contract

Issue #2162 installs the source-owned form, data, feedback, and interaction families required by
the migration: Button, Input, Textarea, Checkbox, RadioGroup, Switch, Slider, Select, Combobox
composition, Toggle/ToggleGroup, InputGroup, Field/FieldSet, Card, Table, Badge, Avatar, Alert,
Empty, Skeleton, Spinner, Progress, Separator, toast, Accordion, Collapsible, Resizable,
Pagination, ContextMenu, and HoverCard. Generated registry components remain unmodified except for
the reviewed semantic overlay and mobile-sidebar accessibility fixes recorded above.

The approved composed contracts live under `components/cortana`: `AsyncButton` owns busy and
disabled semantics, `ValidatedInput` owns label/help/error association, `StatusBadge` owns busy,
offline, success, warning, and error tones, and `FeedbackState` owns loading, empty, success,
warning, error, and retry presentation. Primary, secondary, outline, ghost, link, destructive,
compact, icon, busy, disabled, and validation behavior is expressed through generated variants,
sizes, and these compositions rather than page-local classes. Ordinary icon-only actions use the
generated icon sizes and always require an accessible name at the call site.

## Legacy control and CSS inventory

The production renderer starts with 18 raw `button` elements, 139 uses of the temporary shared
`Button`, 60 raw inputs, seven raw selects, and four raw textareas. The generated shadcn directory
is excluded from those counts. Manual dialog, tablist, tooltip, confirmation, and menu behavior is
owned across `App.tsx`, `Workspace.tsx`, `SettingsView.tsx`, `MemoryReview.tsx`, `SourcePanel.tsx`,
`ContextPanel.tsx`, and `Navigation.tsx`.

The legacy stylesheet contract is 5,005 lines:

| Owner                   | Lines | Current responsibility                                  | Replacement                                                          |
| ----------------------- | ----: | ------------------------------------------------------- | -------------------------------------------------------------------- |
| `styles/tokens.css`     |   354 | Four themes, type, focus, control sizes, reduced motion | semantic variables in `shadcn.css`; #2161 and #2167                  |
| `styles/buttons.css`    |    97 | temporary `cortana-button` variants                     | generated Button variants; #2162 and #2167                           |
| `styles/shell.css`      | 1,068 | title bar, rail, panes, status, overlays                | Sidebar, Sheet, Tooltip, Command, shell composition; #2163 and #2164 |
| `styles/workspace.css`  |   809 | document, answer, graph, timeline, tabs                 | Card, Tabs, ScrollArea, Empty, Skeleton, renderer wrapper; #2165     |
| `styles/context.css`    |   159 | agent-context pane                                      | Card, Sheet, Badge, Progress; #2165                                  |
| `styles/settings.css`   | 1,845 | setup, sources, forms, services, updates                | Field system and shared feedback/overlays; #2162 and #2166           |
| `styles/utility.css`    |   470 | inbox, conversations, agent tools, memory               | shared cards, tables, status, menus; #2165 and #2166                 |
| `styles/responsive.css` |   203 | 1280, 1000, and 780 pixel shell forks                   | component-owned responsive composition; #2164 through #2167          |

Additional page-local breakpoints exist at 760, 780, and 900 pixels. The final renderer may retain
layout breakpoints, but it must not retain a second component styling or interaction system.

## Surface inventory and execution sequence

| Surface or state                                                                                                                                                | Legacy owner                                                                                | shadcn owner                                                                                | Issue |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- | ----- |
| Buttons, inputs, textareas, labels, checkboxes, switches, selects, fields, cards, badges, alerts, progress, skeletons, spinners, empty and error states         | `components/ui/Button.tsx`, raw controls, page-local status markup                          | generated shared primitives and composed variants                                           | #2162 |
| Dialogs, destructive confirmations, sheets, dropdowns, popovers, tooltips, command palette, focus entry/return                                                  | manual markup in `App.tsx`, `SettingsView.tsx`, `MemoryReview.tsx`, and CSS pseudo-tooltips | Dialog, AlertDialog, Sheet, DropdownMenu, Popover, Tooltip, Command                         | #2163 |
| Title/search bar, primary navigation, source navigation, context pane, resizing, status and background activity                                                 | `App.tsx`, `Navigation.tsx`, `SourcePanel.tsx`, `ContextPanel.tsx`                          | responsive Sidebar/Sheet shell and shared status components                                 | #2164 |
| Workspace selection, search, documents, answer, citations, context bundle, graph, timeline, conversations, memory review                                        | `Workspace.tsx`, `MemoryReview.tsx`, `UtilityView.tsx`, `App.tsx`                           | Tabs, Card, Breadcrumb, ScrollArea, Table, menus, feedback, renderer-specific graph wrapper | #2165 |
| First-run setup, source setup, initial sync, service readiness, settings, principals, models, backups, updater, recovery, activity inbox and operational errors | `SettingsView.tsx`, `SourceInitialSync.tsx`, `UtilityView.tsx`                              | Field/FieldGroup forms, shared navigation, progress, alerts, overlays and confirmations     | #2166 |
| Legacy button/classes, eight CSS files, page-local primitives, renderer flag, duplicate theme and responsive contracts                                          | all owners above                                                                            | one generated/composed system                                                               | #2167 |
| Loading, empty, partial, offline, busy, success, warning, destructive, error, responsive, zoom, reduced-motion, keyboard, screen-reader and packaged states     | full renderer and typed native bridge                                                       | final migrated renderer                                                                     | #2168 |

The graph canvas, virtualized document list, native title-bar integration, and native file/dialog
launch points remain explicit renderer exceptions. Their controls, surrounding surfaces, status,
focus, and theme behavior still use shared shadcn composition. No M7 work changes Tauri capabilities,
native commands, source approval, credential handling, retrieval behavior, or stored data.

## Knowledge workspace migration

Issue #2165 keeps retrieval, ACL, pagination, cancellation, virtualization, and graph traversal in
the existing data owners while replacing their presentation boundary in the shadcn renderer. The
live renderer now uses shared Tabs, Button, Input, Select, Toggle, Card, Badge, Empty, ScrollArea,
and Textarea primitives across workspace navigation and feedback, source and document navigation,
the context inspector, bounded graph controls, and native-memory review. The legacy renderer still
receives its original markup and styles through the renderer flag until #2167 removes the rollback
path.

The canonical document body, radial graph canvas, and virtualized list math remain purpose-built.
Their interactive rows, filters, pagination, status, selected-node inspector, theme colors, and
responsive boundaries are shadcn-owned. This preserves stable document provenance and bounded graph
loading without disguising custom renderers as ordinary component primitives.

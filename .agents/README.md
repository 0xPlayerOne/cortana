# Cortana project skills

This directory contains the coding-agent skills that are shared with Cortana
contributors and automation. The set is intentionally curated for Cortana's
Rust core, Python connector boundaries, and Tauri/React UI.

## Included

- `accessibility` and `frontend-ui-engineering` for the desktop/web UX.
- `shadcn` for the planned component-system migration; Cortana does not yet
  have a `components.json`, so the CLI should not be run until that migration
  is explicitly underway.
- `performance-optimization` for measured retrieval, ingestion, and UI work.
- `security-and-hardening` for connectors, auth, ACLs, and personal data.
- `test-driven-development`, `incremental-implementation`, and
  `verification-before-completion` for safe changes.
- `code-review`, `code-simplification`, and `typescript-advanced-types` for
  review and the TypeScript UI surface.

The skills are maintained as source files rather than symlinks so a checkout
is self-contained and reproducible. See `.skill-lock.json` for provenance and
the deliberate exclusions.

## Provenance and refresh

The UI, testing, security, performance, and review skills are synchronized from
the public `adea-ai/agent-hq` skill set. `code-simplification` is
synchronized from `adea-ai/control-plane`. Refreshes must be reviewed as
normal Cortana changes; do not overwrite local adaptations blindly.

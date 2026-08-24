# Planning and tracking

GitHub is Cortana's authoritative planning and delivery system.

## Canonical planning sources

- [Milestones](https://github.com/0xPlayerOne/cortana/milestones) define outcome-oriented delivery groups.
- [Issues](https://github.com/0xPlayerOne/cortana/issues) define issue-sized work, ownership, dependencies, blockers, acceptance criteria, and evidence.
- [Pull requests](https://github.com/0xPlayerOne/cortana/pulls) contain reviewed implementation changes and CI results.
- [Releases](https://github.com/0xPlayerOne/cortana/releases) identify published version boundaries.
- [Release history](releases.md) preserves immutable version-specific verification and incident evidence.

No PRD, TDD, guide, audit, or operations document should duplicate the live milestone sequence or maintain a second backlog.

## What belongs in GitHub

Use a milestone or issue for anything that can change as work progresses:

- current status;
- planned work;
- owner or assignee;
- priority and sequence;
- target dates;
- dependencies and blockers;
- implementation choice still under investigation;
- task-level acceptance criteria;
- validation evidence for a specific change;
- rollout activation or deferral;
- superseded work.

## What belongs in durable documentation

Use durable documents for information that remains true independently of the current backlog:

- product purpose and user promise;
- system ownership and trust boundaries;
- canonical terminology;
- accepted architecture;
- data and interface contracts;
- security invariants;
- source and memory lifecycle rules;
- evaluation methodology;
- operational procedures;
- release-evidence retention rules.

A durable decision that changes ownership, persistence, trust, compatibility, or deployment requires an ADR and corresponding updates to the owning specification and TDD.

## Issue structure

New implementation issues should normally contain:

1. **Outcome** — the user or system result.
2. **Scope** — the bounded work.
3. **Acceptance criteria** — observable completion conditions.
4. **Dependencies** — prerequisite contracts or issues where relevant.
5. **Safety constraints** — operations that are forbidden or approval-gated.

Avoid broad umbrella issues that mix several independently verifiable outcomes. When an old epic is superseded, comment with the replacement map before closing it.

## Evidence model

- CI results prove only the tested source tree and environment.
- A tagged release proves only the published artifact and recorded release checks.
- Static package verification does not prove packaged GUI interaction or operating-system trust.
- Read-only source validation does not authorize ingestion, reconciliation, or scheduling.
- A bounded trial proves only the tested source, scope, and budget.
- Synthetic evaluation does not replace approved-corpus evaluation.
- Memory persistence and recall do not imply consolidation or reflection.
- A feature is complete only when its issue acceptance criteria and required evidence are satisfied.

## Document review

When planning changes:

- remove obsolete milestone prose from documents;
- preserve durable requirements and safety constraints;
- move unresolved work into GitHub;
- link immutable evidence rather than copying it;
- update ADRs and specifications when the accepted architecture changes.

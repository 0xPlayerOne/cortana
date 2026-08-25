# Workspace, project, principal, and ACL mapping

The identity contract is `cortana.identity.v1`. It prevents a UI workspace, Agent HQ workspace, and
Cortana project from being treated as interchangeable identifiers.

## Terms

- **Workspace:** human-facing grouping such as work, personal, or special.
- **Project:** persisted Cortana scope used by documents, memories, sources, and retrieval.
- **Principal:** owner or bearer identity authenticated by a configured credential and scopes.
- **ACL label:** an explicit visibility label attached to canonical records.
- **External execution identity:** an Agent HQ/Control Plane identity mapped to a Cortana principal;
  it is never assumed equal to a Cortana ID.

Each mapping has an opaque local ID, external system/name, target project, principal, allowed scopes,
ACL intersection, mapping version, creation/update timestamps, status, and audit reference. Tokens
are referenced by name only and remain outside the mapping.

## Operations

Creation and rename preserve IDs. Transfer requires owner/admin authorization and a new audited
mapping version. Revocation immediately denies reads/writes and invalidates scoped caches. Deletion
quarantines orphaned records until an explicit owner decision; it never silently reassigns data.

Workspace selection is a filter. Authorization is the intersection of principal scopes, mapping
scope, requested project, and record ACL. This invariant applies to evidence, memories, document
browse, graph, sync status, exports, caches, and Desktop projections.

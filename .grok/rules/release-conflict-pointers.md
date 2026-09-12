# Release notes: conflict pointers

When cutting a release (GitHub release body, annotated tag message, or any
release notes you write for this fork), include a **Conflicts resolved**
pointer section if any merge conflicts were resolved since the previous
release. Skip the section when there were none.

Put it after the usual release notes. Use an unordered list. Each conflicted
file is one item, in this exact shape:

```markdown
## Conflicts resolved

- `path/to/file` &nbsp; short one-line description

   **This Fork** &nbsp; What this fork wanted, and why. May span multiple
   lines.

   **Upstream** &nbsp; What upstream changed in the same place, and why
   it conflicted.

   (Optional) A fenced diff hunk if it helps a reader see the choice.
```

Rules:

- One list item per conflicted file path. Use the repo-relative path in
  backticks.
- The one-line description stays on the same bullet as the path, separated
  by `&nbsp;`.
- **This Fork** and **Upstream** are indented under that bullet. Each label
  is bold, then `&nbsp;`, then the explanation.
- Explanations are prose, not a restatement of the one-liner. Say the
  behavior on each side and which side we kept (or how they were combined).
- Diff hunks are optional. Include them only when the prose is not enough.
- If a file had several hunks but one decision, keep a single list item.

Example:

```markdown
## Conflicts resolved

- `crates/codegen/xai-grok-shell/src/session/merge.rs` &nbsp; Empty-session list vs hide

   **This Fork** &nbsp; The session list shows leftover zero-turn sessions.
   `grok sessions prune` deletes them. Tests cover prune, not hide-on-list.

   **Upstream** &nbsp; Rewrote `dedup_empty_sessions` to drop unnamed empty
   local husks from the list, and added tests for that hide behavior.

- `crates/codegen/xai-grok-pager/src/app/dispatch/tests/jump.rs` &nbsp; Tests added in the same place

   **This Fork** &nbsp; `/jump N` tests: in-range goto, out-of-range toast,
   locate flash, picker dismiss.

   **Upstream** &nbsp; New next/prev-response navigation tests in the same
   file. Kept both sets.
```

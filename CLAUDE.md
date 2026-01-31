# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

# beads-lite

Minimal dependency-aware task tracker for coding agents. Tracks tasks with blocking dependencies and answers "what's ready to work on?"

## Commands

```bash
cargo test            # run tests
cargo build --release # build release binary
cargo run -- <args>   # run cli during dev

# Installed binary usage
bl ready              # what can I work on?
bl list --tree        # see all tasks with dependencies
bl create "title"     # new task
bl close <id>         # complete task
bl update <id> --status in_progress  # claim work
bl update <a> --blocked-by <b>       # a blocked by b
```

## Development Rules

1. **TDD**: Write failing test first, then implement
2. **Key Crates**: `rusqlite` (DB), `clap` (CLI), `serde` (Serialization)
3. **No short flags**: Use `--json` not `-j` (AI agents parse long flags better)
4. **Tests are the spec**: When in doubt, check the tests
5. **Conventional commits**: Required for auto-release (see below)

## Commit Conventions (Auto-Release)

Pushes to main auto-tag releases based on commit prefix:

| Prefix                        | Version Bump   | Example                             |
| --------                      | -------------- | ---------                           |
| `fix:`                        | Patch (0.0.X)  | `fix: handle nil pointer in export` |
| `perf:`                       | Patch (0.0.X)  | `perf: batch dependency queries`    |
| `feat:`                       | Minor (0.X.0)  | `feat: add --filter flag to list`   |
| `feat!:` or `BREAKING CHANGE` | Major (X.0.0)  | `feat!: change ID format`           |

**No release** (docs, tests, chore):
- `docs:`, `test:`, `chore:`, `style:`, `ci:` - no version bump
- Add `[skip release]` to any commit to skip tagging

**Rules:**
- Every functional change (fix/feat/perf) triggers a release
- Keep commits atomic - one logical change per commit
- Reference issue IDs in commit body: `Closes: bl-xxxx`

## Architecture

```
src/main.rs          # Entry point
src/cli/             # CLI command handlers and output formatting
src/issue.rs         # Issue struct + validation + ID generation
src/dependency.rs    # Dependency relationship types
src/storage.rs       # SQLite implementation + blocking algorithm
src/jsonl.rs         # Import/export for git backup + two-phase import
```

## Core Algorithm: Ready Work Calculation

The blocking calculation is in `src/storage.rs:get_ready_work()`. Uses a subquery:

```sql
SELECT i.id, i.title, ...
FROM issues i
WHERE i.status IN ('open', 'in_progress')
AND i.id NOT IN (
    SELECT DISTINCT d.issue_id
    FROM dependencies d
    JOIN issues blocker ON d.depends_on_id = blocker.id
    WHERE d.type = 'blocks'
      AND blocker.status != 'closed'
)
ORDER BY i.priority ASC, i.created_at ASC
```

**Logic**: An issue is "ready" if:
1. Status is `open` or `in_progress`
2. NOT blocked by any open issue (via `blocks` dependency)

## Key Implementation Details

### ID Generation (Hash-based)

Uses SHA-256 + base36 encoding for compact IDs (`src/issue.rs`):
- Format: `bl-{4-char-base36}` (e.g., `bl-g9d5`)
- Input: title + description + timestamp nanoseconds
- Uses `num_bigint` for base36 conversion

### Two-Phase Import Pattern

JSONL import handles forward references (`src/jsonl.rs`):
1. **Phase 1**: Create/update all issues (without dependencies)
2. **Phase 2**: Clear old dependencies, add new ones

This prevents partial state when an issue depends on another that appears later in the file.

### Transaction Pattern

`storage.with_transaction()` wraps operations in `BEGIN IMMEDIATE` / `COMMIT` with automatic rollback on error.

### Output Modes

Three output formats for list/ready:
- **Default**: Human-readable table format
- **JSONL**: Machine-readable (one JSON per line) for git backup
- **Tree**: Dependency tree visualization (roots = issues not blocked by open issues)

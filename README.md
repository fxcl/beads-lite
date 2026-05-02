# beads-lite

Minimal dependency-aware task tracker for coding agents.

## Install

```bash
# macOS/Linux
curl -sSL https://raw.githubusercontent.com/fxcl/beads-lite/main/install.sh | sh

```

## Usage

```bash
bl init                    # initialize in current directory
bl create "Fix login bug"  # create a task
bl ready                   # what can I work on?
bl close <id>              # complete a task
```

### Dependencies

```bash
bl create "Deploy"
bl create "Write tests"
bl update <deploy-id> --blocked-by <tests-id>  # deploy blocked until tests done
bl ready                                        # only "Write tests" shows
bl close <tests-id>
bl ready                                        # now "Deploy" shows
```

### Key-based Workflow

Instead of relying on generated IDs, you can define and use semantic keys. This creates a much smoother workflow for LLM agents:

```bash
# 1. Create a task with a specific key
bl create --key auth-api "Implement Auth API"
# The title will automatically be saved as: "[auth-api] Implement Auth API"

# 2. Operate on the task using the key instead of the ID
bl show --key auth-api
bl update --key auth-api --status in_progress
bl close --key auth-api --resolution done
```

Any command that accepts an `<id>` also accepts `--key <key>` as an alternative.

### Task Origin Tracking

Use `--discovered-from` to track where a task was discovered:

```bash
bl create "Implement authentication"
# While working on auth, discover a need to refactor the database
bl create "Refactor user schema" --discovered-from <auth-id>
bl show <refactor-id>  # shows "Discovered From: <auth-id>"

# Or add to existing task
bl update <existing-task-id> --discovered-from <source-id>
```

Unlike `--blocked-by`, discovered-from tasks appear in `bl ready` immediately — they record context, not execution order.

### CLI Reference

```
Usage: bl <command> [args]

Commands:
  init                  Initialize .beads-lite/ directory and database
  create <title>        Create a new issue, prints ID
  list                  List all issues
  show <id|--key>       Show issue details
  update <id|--key>     Update an issue (including blockers)
  delete <id|--key>     Delete an issue permanently (requires --confirm)
  close <id|--key>      Close an issue
  ready                 List unblocked work
  export [file]         Export all issues to JSONL (stdout or file)
  import <file>         Import issues from JSONL file
  onboard               Print Claude Code integration instructions
  version               Show version
  upgrade               Upgrade to latest release

List/Ready Flags:
  --json                Output as JSONL (one JSON object per line)
  --tree                Show dependency tree
  --priority <int>      Filter by priority (0-4)
  --type <string>       Filter by type (task, bug, feature, epic)

List-Only Flags:
  --status <string>     Filter by status (open, in_progress, closed)
  --resolution <string> Filter by resolution (done, wontfix, duplicate)

Show Flags:
  --json                Output as JSON

Create Flags:
  --key <string>        Issue key to prepend to title (e.g., [key])
  --description <text>  Issue description
  --priority <int>      Priority (0-4), default 2
  --type <string>       Type (task, bug, feature, epic), default task
  --blocked-by <id>     Issue ID that blocks this (repeatable)
  --discovered-from <id> Issue ID this was discovered from (repeatable)

Update Flags:
  --title <string>      New title
  --status <string>     New status (open, in_progress, closed)
  --priority <int>      New priority (0-4)
  --type <string>       New type (task, bug, feature, epic)
  --description <text>  New description
  --blocked-by <id>     Add blocker (repeatable)
  --unblock <id>        Remove blocker (repeatable)
  --discovered-from <id> Add discovered-from link (repeatable)

Close Flags:
  --resolution <string> Resolution (done, wontfix, duplicate), default done

Delete Flags:
  --confirm             Required to confirm permanent deletion
```

## Development

```bash
just test   # run tests
just build  # build ./bl binary
```

## License

MIT

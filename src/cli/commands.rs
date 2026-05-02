//! CLI command handlers.

use crate::cli::output::{output_issues, output_single_issue_json};
use crate::dependency::DepType;
use crate::issue::{Issue, IssueType, Resolution, Status};
use crate::jsonl::{export_to_file, export_to_jsonl, import_from_file};
use crate::storage::Store;
use crate::sync::SyncEngine;
use clap::{Parser, Subcommand};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

const BEADS_DIR: &str = ".beads-lite";
const DB_NAME: &str = "beads.db";

/// Version is set at build time via env or default
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(name = "bl", about = "Minimal dependency-aware task tracker for coding agents")]
pub struct Cli {
    /// Show version
    #[arg(short = 'v', long = "version")]
    pub version_flag: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize .beads-lite/ directory and database
    Init,
    /// Create a new issue, prints ID
    Create {
        /// Issue key to prepend to the title (e.g., [key])
        #[arg(long)]
        key: Option<String>,
        /// Issue title
        title: Vec<String>,
        /// Issue description
        #[arg(long)]
        description: Option<String>,
        /// Priority (0-4)
        #[arg(long, default_value_t = 2)]
        priority: i32,
        /// Type (task, bug, feature, epic)
        #[arg(long, rename_all = "lowercase", default_value = "task")]
        r#type: String,
        /// Issue ID that blocks this (repeatable)
        #[arg(long = "blocked-by")]
        blocked_by: Vec<String>,
        /// Issue ID this was discovered from (repeatable)
        #[arg(long = "discovered-from")]
        discovered_from: Vec<String>,
    },
    /// List all issues
    List {
        /// Output as JSONL
        #[arg(long)]
        json: bool,
        /// Show dependency tree
        #[arg(long)]
        tree: bool,
        /// Filter by status
        #[arg(long)]
        status: Option<String>,
        /// Filter by priority (0-4)
        #[arg(long)]
        priority: Option<i32>,
        /// Filter by type
        #[arg(long)]
        r#type: Option<String>,
        /// Filter by resolution
        #[arg(long)]
        resolution: Option<String>,
        /// Filter by blocker (issue ID)
        #[arg(long = "blocked-by")]
        blocked_by: Option<String>,
    },
    /// Show issue details
    Show {
        /// Issue ID
        #[arg(required_unless_present = "key", conflicts_with = "key")]
        id: Option<String>,
        /// Issue key (alternative to ID, prefix in brackets)
        #[arg(long, conflicts_with = "id")]
        key: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Update an issue
    Update {
        /// Issue ID
        #[arg(required_unless_present = "key", conflicts_with = "key")]
        id: Option<String>,
        /// Issue key (alternative to ID, prefix in brackets)
        #[arg(long, conflicts_with = "id")]
        key: Option<String>,
        /// New title
        #[arg(long)]
        title: Option<String>,
        /// New status
        #[arg(long)]
        status: Option<String>,
        /// New priority (0-4)
        #[arg(long)]
        priority: Option<i32>,
        /// New type
        #[arg(long)]
        r#type: Option<String>,
        /// New description
        #[arg(long)]
        description: Option<String>,
        /// Add blocker (repeatable)
        #[arg(long = "blocked-by")]
        blocked_by: Vec<String>,
        /// Remove blocker (repeatable)
        #[arg(long)]
        unblock: Vec<String>,
        /// Add discovered-from link (repeatable)
        #[arg(long = "discovered-from")]
        discovered_from: Vec<String>,
    },
    /// Delete an issue permanently
    Delete {
        /// Issue ID
        #[arg(required_unless_present = "key", conflicts_with = "key")]
        id: Option<String>,
        /// Issue key (alternative to ID, prefix in brackets)
        #[arg(long, conflicts_with = "id")]
        key: Option<String>,
        /// Confirm deletion
        #[arg(long)]
        confirm: bool,
    },
    /// Close an issue
    Close {
        /// Issue ID
        #[arg(required_unless_present = "key", conflicts_with = "key")]
        id: Option<String>,
        /// Issue key (alternative to ID, prefix in brackets)
        #[arg(long, conflicts_with = "id")]
        key: Option<String>,
        /// Resolution (done, wontfix, duplicate)
        #[arg(long, default_value = "done")]
        resolution: String,
        /// Reason for closing
        #[arg(long)]
        reason: Option<String>,
    },
    /// List unblocked work
    Ready {
        /// Output as JSONL
        #[arg(long)]
        json: bool,
        /// Show dependency tree
        #[arg(long)]
        tree: bool,
        /// Filter by priority (0-4)
        #[arg(long)]
        priority: Option<i32>,
        /// Filter by type
        #[arg(long)]
        r#type: Option<String>,
        /// Limit number of results
        #[arg(short = 'n', long)]
        limit: Option<usize>,
    },
    /// Export all issues to JSONL
    Export {
        /// Output file (stdout if not specified)
        file: Option<PathBuf>,
    },
    /// Import issues from JSONL file
    Import {
        /// Input file
        file: PathBuf,
    },
    /// Synchronize issues with code
    Sync,
    /// Print Claude Code integration instructions
    Onboard,
    /// Show version
    Version,
    /// Upgrade to latest release
    Upgrade,
}

fn get_db_path() -> PathBuf {
    PathBuf::from(BEADS_DIR).join(DB_NAME)
}

fn resolve_id(store: &Store, id: Option<String>, key: Option<String>) -> Result<String, String> {
    if let Some(id) = id {
        Ok(id)
    } else if let Some(key) = key {
        let issue = store.get_issue_by_key(&key).map_err(|e| format!("key {}: {}", key, e))?;
        Ok(issue.id)
    } else {
        Err("either id or --key must be provided".to_string())
    }
}

fn open_store() -> Result<Store, String> {
    let db_path = get_db_path();
    if !db_path.exists() {
        return Err("not initialized: run 'bl init' first".to_string());
    }
    Store::new(&db_path).map_err(|e| format!("failed to open database: {}", e))
}

/// Runs the CLI with the given command.
pub fn run<W: Write>(cli: Cli, writer: &mut W) -> Result<(), String> {
    // Handle -v/--version flag
    if cli.version_flag {
        return cmd_version(writer);
    }

    match cli.command {
        None => {
            print_help(writer);
            Ok(())
        }
        Some(cmd) => run_command(cmd, writer),
    }
}

fn run_command<W: Write>(cmd: Commands, writer: &mut W) -> Result<(), String> {
    match cmd {
        Commands::Init => cmd_init(writer),
        Commands::Create {
            key,
            title,
            description,
            priority,
            r#type,
            blocked_by,
            discovered_from,
        } => {
            let mut final_title = title.clone();
            if let Some(k) = key {
                final_title.insert(0, format!("[{}]", k));
            }
            let store = open_store()?;
            cmd_create(store, final_title, description, priority, r#type, blocked_by, discovered_from, writer)
        }
        Commands::List {
            json,
            tree,
            status,
            priority,
            r#type,
            resolution,
            blocked_by,
        } => {
            let store = open_store()?;
            cmd_list(store, json, tree, status, priority, r#type, resolution, blocked_by, writer)
        }
        Commands::Show { id, key, json } => {
            let store = open_store()?;
            let resolved_id = resolve_id(&store, id, key)?;
            cmd_show(store, resolved_id, json, writer)
        }
        Commands::Update {
            id,
            key,
            title,
            status,
            priority,
            r#type,
            description,
            blocked_by,
            unblock,
            discovered_from,
        } => {
            let store = open_store()?;
            let resolved_id = resolve_id(&store, id, key)?;
            cmd_update(
                store,
                resolved_id,
                title,
                status,
                priority,
                r#type,
                description,
                blocked_by,
                unblock,
                discovered_from,
                writer,
            )
        }
        Commands::Delete { id, key, confirm } => {
            let store = open_store()?;
            let resolved_id = resolve_id(&store, id, key)?;
            cmd_delete(store, resolved_id, confirm, writer)
        }
        Commands::Close { id, key, resolution, reason } => {
            let store = open_store()?;
            let resolved_id = resolve_id(&store, id, key)?;
            cmd_close(store, resolved_id, resolution, reason, writer)
        }
        Commands::Ready {
            json,
            tree,
            priority,
            r#type,
            limit,
        } => {
            let store = open_store()?;
            cmd_ready(store, json, tree, priority, r#type, limit, writer)
        }
        Commands::Export { file } => {
            let store = open_store()?;
            cmd_export(store, file, writer)
        }
        Commands::Import { file } => {
            let store = open_store()?;
            cmd_import(store, &file, writer)
        }
        Commands::Sync => {
            let store = open_store()?;
            cmd_sync(store, writer)
        }
        Commands::Onboard => cmd_onboard(writer),
        Commands::Version => cmd_version(writer),
        Commands::Upgrade => cmd_upgrade(writer),
    }
}

fn print_help<W: Write>(writer: &mut W) {
    let _ = writeln!(
        writer,
        r#"Usage: bl <command> [args]

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
  sync                  Synchronize issues with code
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
  --status <string>     Filter by status (open, in_progress, closed)
  --resolution <string> Filter by resolution (done, wontfix, duplicate)
  --blocked-by <id>     Filter by blocker (issues blocked by <id>)

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
  --reason <text>       Reason for closing

Delete Flags:
  --confirm             Required to confirm permanent deletion"#
    );
}

fn cmd_init<W: Write>(writer: &mut W) -> Result<(), String> {
    fs::create_dir_all(BEADS_DIR).map_err(|e| format!("failed to create {}: {}", BEADS_DIR, e))?;
    Store::new(get_db_path()).map_err(|e| format!("failed to initialize database: {}", e))?;
    writeln!(writer, "Initialized beads-lite in {}", BEADS_DIR).map_err(|e| e.to_string())?;
    writeln!(writer).map_err(|e| e.to_string())?;
    writeln!(
        writer,
        "Tip: Run 'bl onboard > .claude/CLAUDE.md' to set up Claude Code integration"
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_create<W: Write>(
    store: Store,
    title: Vec<String>,
    description: Option<String>,
    priority: i32,
    issue_type: String,
    blocked_by: Vec<String>,
    discovered_from: Vec<String>,
    writer: &mut W,
) -> Result<(), String> {
    if title.is_empty() {
        return Err("usage: bl create <title> [--description <text>] [--priority <0-4>] [--type <task|bug|feature|epic>] [--blocked-by <id>] [--discovered-from <id>]".to_string());
    }

    let title = title.join(" ");

    let mut issue = Issue::new(&title);
    issue.description = description.unwrap_or_default();
    issue.priority = priority;
    issue.issue_type = issue_type
        .parse::<IssueType>()
        .map_err(|_| format!("invalid type: {} (valid: task, bug, feature, epic)", issue_type))?;

    store.create_issue(&issue).map_err(|e| format!("failed to create issue: {}", e))?;

    // Add blockers
    for blocker_id in &blocked_by {
        if blocker_id == &issue.id {
            return Err("issue cannot block itself".to_string());
        }
        store
            .get_issue(blocker_id)
            .map_err(|e| format!("blocker issue {}: {}", blocker_id, e))?;
        store
            .add_dependency(&issue.id, blocker_id, DepType::Blocks)
            .map_err(|e| format!("blocker issue {}: {}", blocker_id, e))?;
    }

    // Add discovered-from links
    for source_id in &discovered_from {
        if source_id == &issue.id {
            return Err("issue cannot be discovered from itself".to_string());
        }
        store
            .get_issue(source_id)
            .map_err(|e| format!("source issue {}: {}", source_id, e))?;
        store
            .add_dependency(&issue.id, source_id, DepType::DiscoveredFrom)
            .map_err(|e| format!("source issue {}: {}", source_id, e))?;
    }

    writeln!(writer, "Created {}: {}", issue.id, issue.title).map_err(|e| e.to_string())?;
    Ok(())
}

fn validate_filters(
    status: &Option<String>,
    priority: &Option<i32>,
    issue_type: &Option<String>,
    resolution: &Option<String>,
) -> Result<(), String> {
    if let Some(s) = status {
        if s.parse::<Status>().is_err() {
            return Err(format!("invalid status: {} (valid: open, in_progress, closed)", s));
        }
    }
    if let Some(p) = priority {
        if !(*p >= 0 && *p <= 4) {
            return Err(format!("invalid priority: {} (valid: 0-4)", p));
        }
    }
    if let Some(t) = issue_type {
        if t.parse::<IssueType>().is_err() {
            return Err(format!("invalid type: {} (valid: task, bug, feature, epic)", t));
        }
    }
    if let Some(r) = resolution {
        if r.parse::<Resolution>().is_err() {
            return Err(format!("invalid resolution: {} (valid: done, wontfix, duplicate)", r));
        }
    }
    Ok(())
}

fn filter_issues(
    issues: Vec<Issue>,
    status: &Option<String>,
    priority: &Option<i32>,
    issue_type: &Option<String>,
    resolution: &Option<String>,
) -> Vec<Issue> {
    issues
        .into_iter()
        .filter(|i| {
            if let Some(s) = status {
                if i.status.as_str() != s {
                    return false;
                }
            }
            if let Some(p) = priority {
                if i.priority != *p {
                    return false;
                }
            }
            if let Some(t) = issue_type {
                if i.issue_type.as_str() != t {
                    return false;
                }
            }
            if let Some(r) = resolution {
                if i.resolution.as_str() != r {
                    return false;
                }
            }
            true
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn cmd_list<W: Write>(
    store: Store,
    json: bool,
    tree: bool,
    status: Option<String>,
    priority: Option<i32>,
    issue_type: Option<String>,
    resolution: Option<String>,
    blocked_by: Option<String>,
    writer: &mut W,
) -> Result<(), String> {
    validate_filters(&status, &priority, &issue_type, &resolution)?;

    let issues = if let Some(blocker_id) = blocked_by {
        store
            .get_blocked_by(&blocker_id)
            .map_err(|e| format!("failed to get blocked issues: {}", e))?
    } else {
        store.list_issues().map_err(|e| format!("failed to list issues: {}", e))?
    };

    let issues = filter_issues(issues, &status, &priority, &issue_type, &resolution);
    output_issues(&store, &issues, writer, json, tree).map_err(|e| e.to_string())?;
    Ok(())
}

fn cmd_show<W: Write>(store: Store, id: String, json: bool, writer: &mut W) -> Result<(), String> {
    let issue = store.get_issue(&id).map_err(|e| format!("issue {}: {}", id, e))?;

    if json {
        let deps = store.get_dependencies(&id).unwrap_or_default();
        output_single_issue_json(&issue, &deps, writer).map_err(|e| e.to_string())?;
        return Ok(());
    }

    writeln!(writer, "ID:       {}", issue.id).map_err(|e| e.to_string())?;
    writeln!(writer, "Title:    {}", issue.title).map_err(|e| e.to_string())?;
    writeln!(writer, "Status:   {}", issue.status).map_err(|e| e.to_string())?;
    writeln!(writer, "Priority: P{}", issue.priority).map_err(|e| e.to_string())?;
    writeln!(writer, "Type:     {}", issue.issue_type).map_err(|e| e.to_string())?;
    if !issue.description.is_empty() {
        writeln!(writer, "Description: {}", issue.description).map_err(|e| e.to_string())?;
    }
    writeln!(writer, "Created:  {}", issue.created_at.format("%Y-%m-%d %H:%M:%S")).map_err(|e| e.to_string())?;
    writeln!(writer, "Updated:  {}", issue.updated_at.format("%Y-%m-%d %H:%M:%S")).map_err(|e| e.to_string())?;
    if let Some(closed) = issue.closed_at {
        writeln!(writer, "Closed:   {}", closed.format("%Y-%m-%d %H:%M:%S")).map_err(|e| e.to_string())?;
    }
    if !matches!(issue.resolution, Resolution::None) {
        writeln!(writer, "Resolution: {}", issue.resolution).map_err(|e| e.to_string())?;
    }
    if let Some(reason) = &issue.close_reason {
        writeln!(writer, "Close Reason: {}", reason).map_err(|e| e.to_string())?;
    }

    // Show dependencies, grouped by type
    if let Ok(deps) = store.get_dependencies(&id) {
        if !deps.is_empty() {
            let mut blockers = Vec::new();
            let mut discovered_from = Vec::new();

            for dep in deps {
                match dep.dep_type {
                    DepType::Blocks => blockers.push(dep),
                    DepType::DiscoveredFrom => discovered_from.push(dep),
                }
            }

            if !blockers.is_empty() {
                writeln!(writer, "\nBlockers:").map_err(|e| e.to_string())?;
                for dep in blockers {
                    if let Ok(blocker) = store.get_issue(&dep.depends_on_id) {
                        writeln!(writer, "  - {}: {}", dep.depends_on_id, blocker.title).map_err(|e| e.to_string())?;
                    } else {
                        writeln!(writer, "  - {}", dep.depends_on_id).map_err(|e| e.to_string())?;
                    }
                }
            }

            if !discovered_from.is_empty() {
                writeln!(writer, "\nDiscovered From:").map_err(|e| e.to_string())?;
                for dep in discovered_from {
                    if let Ok(source) = store.get_issue(&dep.depends_on_id) {
                        writeln!(writer, "  - {}: {}", dep.depends_on_id, source.title).map_err(|e| e.to_string())?;
                    } else {
                        writeln!(writer, "  - {}", dep.depends_on_id).map_err(|e| e.to_string())?;
                    }
                }
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_update<W: Write>(
    store: Store,
    id: String,
    title: Option<String>,
    status: Option<String>,
    priority: Option<i32>,
    issue_type: Option<String>,
    description: Option<String>,
    blocked_by: Vec<String>,
    unblock: Vec<String>,
    discovered_from: Vec<String>,
    writer: &mut W,
) -> Result<(), String> {
    let mut issue = store.get_issue(&id).map_err(|e| format!("issue {}: {}", id, e))?;

    // Validate inputs
    if let Some(ref s) = status {
        if s.parse::<Status>().is_err() {
            return Err(format!("invalid status: {} (valid: open, in_progress, closed)", s));
        }
    }
    if let Some(p) = priority {
        if !(0..=4).contains(&p) {
            return Err(format!("invalid priority: {} (valid: 0-4)", p));
        }
    }
    if let Some(ref t) = issue_type {
        if t.parse::<IssueType>().is_err() {
            return Err(format!("invalid type: {} (valid: task, bug, feature, epic)", t));
        }
    }

    // Apply updates
    if let Some(t) = title {
        issue.title = t;
    }
    if let Some(s) = status {
        issue.status = s.parse().unwrap();
    }
    if let Some(p) = priority {
        issue.priority = p;
    }
    if let Some(t) = issue_type {
        issue.issue_type = t.parse().unwrap();
    }
    if let Some(d) = description {
        issue.description = d;
    }

    store.update_issue(&issue).map_err(|e| format!("failed to update: {}", e))?;

    // Handle blocker additions
    for blocker_id in &blocked_by {
        if blocker_id == &id {
            return Err("issue cannot block itself".to_string());
        }
        store
            .get_issue(blocker_id)
            .map_err(|e| format!("blocker issue {}: {}", blocker_id, e))?;
        store
            .add_dependency(&id, blocker_id, DepType::Blocks)
            .map_err(|e| format!("blocker issue {}: {}", blocker_id, e))?;
    }

    // Handle blocker removals
    for blocker_id in &unblock {
        store
            .remove_dependency(&id, blocker_id, DepType::Blocks)
            .map_err(|e| format!("blocker issue {}: {}", blocker_id, e))?;
    }

    // Add discovered-from links
    for source_id in &discovered_from {
        if source_id == &id {
            return Err("issue cannot be discovered from itself".to_string());
        }
        store
            .get_issue(source_id)
            .map_err(|e| format!("source issue {}: {}", source_id, e))?;
        store
            .add_dependency(&id, source_id, DepType::DiscoveredFrom)
            .map_err(|e| format!("source issue {}: {}", source_id, e))?;
    }

    writeln!(writer, "Updated {}: {}", id, issue.title).map_err(|e| e.to_string())?;
    Ok(())
}

fn cmd_delete<W: Write>(store: Store, id: String, confirm: bool, writer: &mut W) -> Result<(), String> {
    if !confirm {
        return Err("delete requires --confirm flag".to_string());
    }

    let issue = store.get_issue(&id).map_err(|e| format!("issue {}: {}", id, e))?;
    store.delete_issue(&id).map_err(|e| format!("failed to delete: {}", e))?;
    writeln!(writer, "Deleted {}: {}", id, issue.title).map_err(|e| e.to_string())?;
    Ok(())
}

fn cmd_close<W: Write>(store: Store, id: String, resolution: String, reason: Option<String>, writer: &mut W) -> Result<(), String> {
    let res = resolution
        .parse::<Resolution>()
        .map_err(|_| format!("invalid resolution: {} (must be done, wontfix, or duplicate)", resolution))?;

    let issue = store.get_issue(&id).map_err(|e| format!("issue {}: {}", id, e))?;
    store.close_issue(&id, res, reason).map_err(|e| format!("failed to close: {}", e))?;
    writeln!(writer, "Closed {}: {}", id, issue.title).map_err(|e| e.to_string())?;
    Ok(())
}

fn cmd_ready<W: Write>(
    store: Store,
    json: bool,
    tree: bool,
    priority: Option<i32>,
    issue_type: Option<String>,
    limit: Option<usize>,
    writer: &mut W,
) -> Result<(), String> {
    validate_filters(&None, &priority, &issue_type, &None)?;

    let issues = store.get_ready_work().map_err(|e| format!("failed to get ready work: {}", e))?;
    let mut issues = filter_issues(issues, &None, &priority, &issue_type, &None);

    if let Some(n) = limit {
        issues.truncate(n);
    }

    output_issues(&store, &issues, writer, json, tree).map_err(|e| e.to_string())?;
    Ok(())
}

fn cmd_export<W: Write>(store: Store, file: Option<PathBuf>, writer: &mut W) -> Result<(), String> {
    if let Some(path) = file {
        export_to_file(&store, &path).map_err(|e| format!("export failed: {}", e))?;
        writeln!(writer, "Exported to {}", path.display()).map_err(|e| e.to_string())?;
    } else {
        export_to_jsonl(&store, writer).map_err(|e| format!("export failed: {}", e))?;
    }
    Ok(())
}

fn cmd_import<W: Write>(mut store: Store, file: &PathBuf, writer: &mut W) -> Result<(), String> {
    let stats = import_from_file(&mut store, file).map_err(|e| format!("import failed: {}", e))?;
    writeln!(writer, "Imported: {} created, {} updated", stats.created, stats.updated).map_err(|e| e.to_string())?;
    Ok(())
}

fn cmd_sync<W: Write>(store: Store, writer: &mut W) -> Result<(), String> {
    // Current directory is assumed to be the repo root or inside it
    let cwd = std::env::current_dir().map_err(|e| format!("failed to get current directory: {}", e))?;
    // We could try to find the git root, but for now assuming cwd is ok or using store's path
    // Store path is usually .beads-lite/beads.db
    // So the repo root is the parent of .beads-lite
    let repo_path = cwd;

    let mut engine = SyncEngine::new(store, repo_path);
    if let Err(e) = engine.run() {
        writeln!(writer, "Sync failed: {}", e).map_err(|e| e.to_string())?;
        return Err("Sync failed".to_string());
    }
    writeln!(writer, "Sync completed successfully.").map_err(|e| e.to_string())?;
    Ok(())
}

fn cmd_onboard<W: Write>(writer: &mut W) -> Result<(), String> {
    let instructions = r#"# beads-lite

This project uses beads-lite for task tracking. You MUST use it to track work.

## Required Workflow

1. Run `bl ready` at session start to see available work
2. When you start working on a task: `bl update <id> --status in_progress`
3. When you discover new work, create a task: `bl create "description"`
4. When tasks depend on each other: `bl update <id> --blocked-by <blocker>`
5. When you complete work: `bl close <id>`

## Commands

```
bl ready              # what can I work on now?
bl ready --json       # machine-readable output
bl list               # all tasks
bl list --tree        # dependency visualization
bl list --status in_progress  # see what's being worked on
bl create "title"     # new task
bl update <id> --status in_progress  # claim work
bl close <id>         # complete task (resolution: done)
bl close <id> --resolution wontfix   # close as won't fix
bl close <id> --resolution duplicate # close as duplicate
bl update <a> --blocked-by <b>       # a blocked by b
bl show <id>          # task details
bl list --status closed --resolution wontfix  # filter by resolution
```

## Closing Tasks

When closing tasks, specify WHY with --resolution:
- `done` (default): Work completed successfully
- `wontfix`: Intentionally rejected (document reasoning in description)
- `duplicate`: Duplicate of another issue

Use `bl list --status closed --resolution wontfix` to review rejected ideas.

## Epic Workflow

Epics group related tasks. Use blockers for actual work dependencies, not organization.

```
# Create epic to track a feature
bl create "User authentication" --type epic

# Create tasks for the epic (work on them immediately)
bl create "Add login endpoint"
bl create "Add session storage"
bl create "Add logout endpoint"

# If tasks have real dependencies, add blockers
bl update <logout-id> --blocked-by <login-id>

# View all work
bl list --tree

# Close tasks as completed, close epic when feature is done
bl close <epic-id>
```

## Rules

- Always check `bl ready` before starting work
- Mark tasks `in_progress` when you start working on them
- Create tasks for any new work you discover
- Close tasks when complete - this unblocks dependent tasks
- Use `--json` flag when you need to parse output programmatically
"#;
    write!(writer, "{}", instructions).map_err(|e| e.to_string())?;
    Ok(())
}

fn cmd_version<W: Write>(writer: &mut W) -> Result<(), String> {
    writeln!(writer, "bl version {}", VERSION).map_err(|e| e.to_string())?;
    Ok(())
}

fn cmd_upgrade<W: Write>(writer: &mut W) -> Result<(), String> {
    const REPO: &str = "fxcl/beads-lite";

    // Get latest release version
    let url = format!("https://api.github.com/repos/{}/releases/latest", REPO);
    let response = ureq::get(&url)
        .header("User-Agent", "beads-lite")
        .call()
        .map_err(|e| format!("failed to check for updates: {}", e))?;

    let release: serde_json::Value =
        serde_json::from_reader(response.into_body().into_reader()).map_err(|e| format!("failed to parse release info: {}", e))?;

    let latest = release["tag_name"].as_str().ok_or("no tag_name in release")?;

    if latest == VERSION {
        writeln!(writer, "Already at latest version {}", VERSION).map_err(|e| e.to_string())?;
        return Ok(());
    }

    writeln!(writer, "Upgrading from {} to {}...", VERSION, latest).map_err(|e| e.to_string())?;

    // Determine platform
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let tarball = format!("beads-lite_{}_{}.tar.gz", os, arch);
    let download_url = format!("https://github.com/{}/releases/download/{}/{}", REPO, latest, tarball);

    // Download tarball
    let response = ureq::get(&download_url).call().map_err(|e| format!("failed to download: {}", e))?;

    // Get current executable path
    let exec_path = std::env::current_exe().map_err(|e| format!("failed to get executable path: {}", e))?;
    let exec_path = exec_path
        .canonicalize()
        .map_err(|e| format!("failed to resolve executable path: {}", e))?;

    // Create temp file for tarball
    let tmp_dir = std::env::temp_dir();
    let tmp_file = tmp_dir.join(format!("bl-upgrade-{}.tar.gz", std::process::id()));

    // Write tarball to temp file
    let mut file = fs::File::create(&tmp_file).map_err(|e| format!("failed to create temp file: {}", e))?;
    let mut reader = response.into_body().into_reader();
    io::copy(&mut reader, &mut file).map_err(|e| format!("failed to download: {}", e))?;
    drop(file);

    // Extract tarball
    let file = fs::File::open(&tmp_file).map_err(|e| format!("failed to open temp file: {}", e))?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);

    let extract_dir = tmp_dir.join(format!("bl-upgrade-extract-{}", std::process::id()));
    fs::create_dir_all(&extract_dir).map_err(|e| format!("failed to create extract dir: {}", e))?;
    archive.unpack(&extract_dir).map_err(|e| format!("failed to extract: {}", e))?;

    // Replace executable
    let new_binary = extract_dir.join("bl");
    fs::copy(&new_binary, &exec_path).map_err(|e| format!("failed to replace executable: {}", e))?;

    // Cleanup
    let _ = fs::remove_file(&tmp_file);
    let _ = fs::remove_dir_all(&extract_dir);

    writeln!(writer, "Upgraded to {}", latest).map_err(|e| e.to_string())?;
    Ok(())
}

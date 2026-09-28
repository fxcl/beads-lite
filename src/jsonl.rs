//! JSONL import/export for git-friendly backups.

use crate::comment::Comment;
use crate::dependency::{DepType, Dependency};
use crate::issue::{Issue, IssueType, Resolution, Status};
use crate::storage::{Store, StoreError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum JsonlError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("line {line}: {message}")]
    ParseLine { line: usize, message: String },
    #[error("store error: {0}")]
    Store(#[from] StoreError),
}

pub type Result<T> = std::result::Result<T, JsonlError>;

/// IssueExport represents an issue with embedded dependencies for JSONL export.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IssueExport {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub status: Status,
    pub priority: i32,
    #[serde(rename = "issue_type")]
    pub issue_type: IssueType,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "is_empty_resolution")]
    pub resolution: Resolution,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_reason: Option<String>,
    pub dependencies: Vec<DependencyExport>,
    /// Embedded comments on the issue. `#[serde(default)]` keeps the import
    /// compatible with JSONL files written before comments existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<CommentExport>,
}

fn is_empty_resolution(r: &Resolution) -> bool {
    matches!(r, Resolution::None)
}

/// DependencyExport represents a dependency relationship for JSONL export.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DependencyExport {
    pub depends_on: String,
    #[serde(rename = "type")]
    pub dep_type: DepType,
}

/// CommentExport is the on-the-wire shape of a [`Comment`] embedded inside
/// an `IssueExport`. Field order matches the [`Comment`] struct.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CommentExport {
    pub id: String,
    pub issue_id: String,
    pub author: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

impl From<&Comment> for CommentExport {
    fn from(c: &Comment) -> Self {
        Self {
            id: c.id.clone(),
            issue_id: c.issue_id.clone(),
            author: c.author.clone(),
            body: c.body.clone(),
            created_at: c.created_at,
        }
    }
}

impl From<CommentExport> for Comment {
    fn from(c: CommentExport) -> Self {
        Self {
            id: c.id,
            issue_id: c.issue_id,
            author: c.author,
            body: c.body,
            created_at: c.created_at,
        }
    }
}

/// ImportStats tracks the results of an import operation.
#[derive(Debug, Default)]
pub struct ImportStats {
    pub created: usize,
    pub updated: usize,
}

/// Converts an Issue, its dependencies, and its comments to an `IssueExport`.
pub fn to_issue_export(issue: &Issue, deps: &[Dependency], comments: &[Comment]) -> IssueExport {
    IssueExport {
        id: issue.id.clone(),
        title: issue.title.clone(),
        description: issue.description.clone(),
        status: issue.status.clone(),
        priority: issue.priority,
        issue_type: issue.issue_type.clone(),
        created_at: issue.created_at,
        updated_at: issue.updated_at,
        closed_at: issue.closed_at,
        resolution: issue.resolution.clone(),
        close_reason: issue.close_reason.clone(),
        dependencies: deps
            .iter()
            .map(|d| DependencyExport {
                depends_on: d.depends_on_id.clone(),
                dep_type: d.dep_type.clone(),
            })
            .collect(),
        comments: comments.iter().map(CommentExport::from).collect(),
    }
}

/// Writes issues with their dependencies and comments to a writer in JSONL format.
pub fn write_issues_as_jsonl<W: Write>(
    issues: &[Issue],
    all_deps: &HashMap<String, Vec<Dependency>>,
    all_comments: &HashMap<String, Vec<Comment>>,
    writer: &mut W,
) -> Result<()> {
    for issue in issues {
        let deps = all_deps.get(&issue.id).map(|v| v.as_slice()).unwrap_or(&[]);
        let comments = all_comments.get(&issue.id).map(|v| v.as_slice()).unwrap_or(&[]);
        let export = to_issue_export(issue, deps, comments);
        serde_json::to_writer(&mut *writer, &export)?;
        writeln!(writer)?;
    }
    Ok(())
}

/// Exports all issues (with deps + comments) to the writer in JSONL format.
pub fn export_to_jsonl<W: Write>(store: &Store, writer: &mut W) -> Result<()> {
    let mut issues = store.list_issues()?;
    let all_deps = store.get_all_dependencies()?;
    let mut all_comments: HashMap<String, Vec<Comment>> = HashMap::new();
    for c in store.list_all_comments()? {
        all_comments.entry(c.issue_id.clone()).or_default().push(c);
    }

    // Sort by ID for deterministic output
    issues.sort_by(|a, b| a.id.cmp(&b.id));

    write_issues_as_jsonl(&issues, &all_deps, &all_comments, writer)
}

/// Exports all issues to the specified file in JSONL format.
pub fn export_to_file<P: AsRef<Path>>(store: &Store, path: P) -> Result<()> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    export_to_jsonl(store, &mut writer)?;
    writer.flush()?;
    Ok(())
}

/// Imports issues from the reader in JSONL format.
/// Uses two-phase import to handle forward references.
pub fn import_from_jsonl<R: BufRead>(store: &mut Store, reader: R) -> Result<ImportStats> {
    let mut stats = ImportStats::default();

    // Pre-scan all lines
    let mut exports = Vec::new();
    for (line_num, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let export: IssueExport = serde_json::from_str(&line).map_err(|e| JsonlError::ParseLine {
            line: line_num + 1,
            message: e.to_string(),
        })?;
        exports.push(export);
    }

    // Process within a transaction
    Ok(store.with_transaction(|conn| {
        // Phase 1: Create/update all issues (without dependencies)
        for (i, export) in exports.iter().enumerate() {
            let line_num = i + 1;

            let issue = Issue {
                id: export.id.clone(),
                title: export.title.clone(),
                description: export.description.clone(),
                status: export.status.clone(),
                priority: export.priority,
                issue_type: export.issue_type.clone(),
                created_at: export.created_at,
                updated_at: export.updated_at,
                closed_at: export.closed_at,
                resolution: export.resolution.clone(),
                close_reason: export.close_reason.clone(),
            };

            // Check if issue exists
            let exists = conn
                .query_row("SELECT 1 FROM issues WHERE id = ?1", [&issue.id], |_| Ok(()))
                .is_ok();

            if exists {
                conn.execute(
                    r#"
                    UPDATE issues SET title = ?1, description = ?2, status = ?3, priority = ?4,
                    issue_type = ?5, updated_at = ?6, closed_at = ?7, resolution = ?8, close_reason = ?9
                    WHERE id = ?10
                    "#,
                    rusqlite::params![
                        issue.title,
                        issue.description,
                        issue.status.as_str(),
                        issue.priority,
                        issue.issue_type.as_str(),
                        issue.updated_at.to_rfc3339(),
                        issue.closed_at.map(|t| t.to_rfc3339()),
                        issue.resolution.as_str(),
                        issue.close_reason,
                        issue.id,
                    ],
                )
                .map_err(|e| StoreError::Validation(format!("line {}: {}", line_num, e)))?;
                stats.updated += 1;
            } else {
                conn.execute(
                    r#"
                    INSERT INTO issues (id, title, description, status, priority, issue_type, created_at, updated_at, closed_at, resolution, close_reason)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                    "#,
                    rusqlite::params![
                        issue.id,
                        issue.title,
                        issue.description,
                        issue.status.as_str(),
                        issue.priority,
                        issue.issue_type.as_str(),
                        issue.created_at.to_rfc3339(),
                        issue.updated_at.to_rfc3339(),
                        issue.closed_at.map(|t| t.to_rfc3339()),
                        issue.resolution.as_str(),
                        issue.close_reason,
                    ],
                )
                .map_err(|e| StoreError::Validation(format!("line {}: {}", line_num, e)))?;
                stats.created += 1;
            }
        }

        // Phase 2: Clear old dependencies + comments, then add new ones
        for (i, export) in exports.iter().enumerate() {
            let line_num = i + 1;

            // Clear existing dependencies
            conn.execute("DELETE FROM dependencies WHERE issue_id = ?1", [&export.id])
                .map_err(|e| StoreError::Validation(format!("line {}: {}", line_num, e)))?;

            // Add new dependencies
            for dep in &export.dependencies {
                conn.execute(
                    r#"
                    INSERT INTO dependencies (issue_id, depends_on_id, type, created_at)
                    VALUES (?1, ?2, ?3, ?4)
                    "#,
                    rusqlite::params![
                        export.id,
                        dep.depends_on,
                        dep.dep_type.as_str(),
                        Utc::now().to_rfc3339(),
                    ],
                )
                .map_err(|e| StoreError::Validation(format!("line {}: {}", line_num, e)))?;
            }

            // Clear existing comments for this issue, then add new ones
            conn.execute("DELETE FROM comments WHERE issue_id = ?1", [&export.id])
                .map_err(|e| StoreError::Validation(format!("line {}: {}", line_num, e)))?;

            for c in &export.comments {
                let c: Comment = c.clone().into();
                conn.execute(
                    r#"
                    INSERT OR IGNORE INTO comments (id, issue_id, author, body, created_at)
                    VALUES (?1, ?2, ?3, ?4, ?5)
                    "#,
                    rusqlite::params![
                        c.id,
                        c.issue_id,
                        c.author,
                        c.body,
                        c.created_at.to_rfc3339(),
                    ],
                )
                .map_err(|e| StoreError::Validation(format!("line {}: {}", line_num, e)))?;
            }
        }

        // Drop comments belonging to issues no longer in the import.
        let keep: Vec<String> = exports.iter().map(|e| e.id.clone()).collect();
        if keep.is_empty() {
            conn.execute("DELETE FROM comments", [])
                .map_err(|e| StoreError::Validation(format!("purge comments: {}", e)))?;
        } else {
            let placeholders: Vec<&str> = std::iter::repeat_n("?", keep.len()).collect();
            let sql = format!(
                "DELETE FROM comments WHERE issue_id NOT IN ({})",
                placeholders.join(",")
            );
            let params: Vec<&dyn rusqlite::ToSql> =
                keep.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
            conn.execute(&sql, params.as_slice())
                .map_err(|e| StoreError::Validation(format!("purge comments: {}", e)))?;
        }

        Ok(stats)
    })?)
}

/// Imports issues from the specified file in JSONL format.
pub fn import_from_file<P: AsRef<Path>>(store: &mut Store, path: P) -> Result<ImportStats> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    import_from_jsonl(store, reader)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_import_roundtrip() {
        let store = Store::in_memory().unwrap();

        let issue_a = Issue::new("Task A");
        let issue_b = Issue::new("Task B");
        store.create_issue(&issue_a).unwrap();
        store.create_issue(&issue_b).unwrap();
        store.add_dependency(&issue_b.id, &issue_a.id, DepType::Blocks).unwrap();

        // Export
        let mut buffer = Vec::new();
        export_to_jsonl(&store, &mut buffer).unwrap();

        // Create new store and import
        let mut new_store = Store::in_memory().unwrap();
        let cursor = std::io::Cursor::new(buffer);
        let stats = import_from_jsonl(&mut new_store, BufReader::new(cursor)).unwrap();

        assert_eq!(stats.created, 2);
        assert_eq!(stats.updated, 0);

        // Verify dependencies preserved
        let ready = new_store.get_ready_work().unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, issue_a.id);
    }
}

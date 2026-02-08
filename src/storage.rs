//! SQLite storage for issues and dependencies.

use crate::dependency::{DepType, Dependency};
use crate::issue::{Issue, IssueType, Resolution, Status};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("issue not found")]
    IssueNotFound,
    #[error("validation error: {0}")]
    Validation(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// Store provides SQLite-backed storage for issues and dependencies.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Creates a new Store with the given database path.
    /// Use ":memory:" for an in-memory database.
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        let store = Store { conn };
        store.init_schema()?;
        Ok(store)
    }

    /// Creates an in-memory database for testing.
    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        Self::new(":memory:")
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS issues (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                description TEXT,
                status TEXT NOT NULL DEFAULT 'open',
                priority INTEGER NOT NULL DEFAULT 2,
                issue_type TEXT NOT NULL DEFAULT 'task',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                closed_at TEXT,
                resolution TEXT
            );

            CREATE TABLE IF NOT EXISTS dependencies (
                issue_id TEXT NOT NULL,
                depends_on_id TEXT NOT NULL,
                type TEXT NOT NULL DEFAULT 'blocks',
                created_at TEXT NOT NULL,
                PRIMARY KEY (issue_id, depends_on_id, type),
                FOREIGN KEY (issue_id) REFERENCES issues(id),
                FOREIGN KEY (depends_on_id) REFERENCES issues(id)
            );

            CREATE INDEX IF NOT EXISTS idx_deps_type ON dependencies(type, depends_on_id);
            CREATE INDEX IF NOT EXISTS idx_issues_status ON issues(status);
            "#,
        )?;

        // Migration: Add close_reason if it doesn't exist
        let _ = self.conn.execute("ALTER TABLE issues ADD COLUMN close_reason TEXT", []);

        Ok(())
    }

    /// Creates a new issue in the database.
    pub fn create_issue(&self, issue: &Issue) -> Result<()> {
        issue.validate().map_err(|e| StoreError::Validation(e.to_string()))?;

        self.conn.execute(
            r#"
            INSERT INTO issues (id, title, description, status, priority, issue_type, created_at, updated_at, closed_at, resolution, close_reason)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
            params![
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
        )?;
        Ok(())
    }

    /// Retrieves an issue by ID.
    pub fn get_issue(&self, id: &str) -> Result<Issue> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, title, description, status, priority, issue_type, created_at, updated_at, closed_at, COALESCE(resolution, ''), close_reason
            FROM issues WHERE id = ?1
            "#,
        )?;

        let issue = stmt.query_row(params![id], |row| {
            let status_str: String = row.get(3)?;
            let type_str: String = row.get(5)?;
            let created_str: String = row.get(6)?;
            let updated_str: String = row.get(7)?;
            let closed_str: Option<String> = row.get(8)?;
            let resolution_str: String = row.get(9)?;

            Ok(Issue {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                status: Status::from_str(&status_str).unwrap_or(Status::Open),
                priority: row.get(4)?,
                issue_type: IssueType::from_str(&type_str).unwrap_or(IssueType::Task),
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .map(|t| t.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                updated_at: DateTime::parse_from_rfc3339(&updated_str)
                    .map(|t| t.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                closed_at: closed_str.and_then(|s| {
                    DateTime::parse_from_rfc3339(&s)
                        .map(|t| t.with_timezone(&Utc))
                        .ok()
                }),
                resolution: Resolution::from_str(&resolution_str).unwrap_or(Resolution::None),
                close_reason: row.get(10)?,
            })
        });

        match issue {
            Ok(i) => Ok(i),
            Err(rusqlite::Error::QueryReturnedNoRows) => Err(StoreError::IssueNotFound),
            Err(e) => Err(StoreError::Database(e)),
        }
    }

    /// Updates an existing issue.
    pub fn update_issue(&self, issue: &Issue) -> Result<()> {
        issue.validate().map_err(|e| StoreError::Validation(e.to_string()))?;

        let updated_at = Utc::now();
        self.conn.execute(
            r#"
            UPDATE issues SET title = ?1, description = ?2, status = ?3, priority = ?4,
            issue_type = ?5, updated_at = ?6, closed_at = ?7, resolution = ?8, close_reason = ?9
            WHERE id = ?10
            "#,
            params![
                issue.title,
                issue.description,
                issue.status.as_str(),
                issue.priority,
                issue.issue_type.as_str(),
                updated_at.to_rfc3339(),
                issue.closed_at.map(|t| t.to_rfc3339()),
                issue.resolution.as_str(),
                issue.close_reason,
                issue.id,
            ],
        )?;
        Ok(())
    }

    /// Closes an issue with the given resolution and optional reason.
    pub fn close_issue(&self, id: &str, resolution: Resolution, reason: Option<String>) -> Result<()> {
        let now = Utc::now();
        self.conn.execute(
            r#"
            UPDATE issues SET status = ?1, updated_at = ?2, closed_at = ?3, resolution = ?4, close_reason = ?5
            WHERE id = ?6
            "#,
            params![
                Status::Closed.as_str(),
                now.to_rfc3339(),
                now.to_rfc3339(),
                resolution.as_str(),
                reason,
                id,
            ],
        )?;
        Ok(())
    }

    /// Lists all issues.
    pub fn list_issues(&self) -> Result<Vec<Issue>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, title, description, status, priority, issue_type, created_at, updated_at, closed_at, COALESCE(resolution, ''), close_reason
            FROM issues ORDER BY priority ASC, created_at ASC
            "#,
        )?;

        let issues = stmt.query_map([], |row| {
            let status_str: String = row.get(3)?;
            let type_str: String = row.get(5)?;
            let created_str: String = row.get(6)?;
            let updated_str: String = row.get(7)?;
            let closed_str: Option<String> = row.get(8)?;
            let resolution_str: String = row.get(9)?;

            Ok(Issue {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                status: Status::from_str(&status_str).unwrap_or(Status::Open),
                priority: row.get(4)?,
                issue_type: IssueType::from_str(&type_str).unwrap_or(IssueType::Task),
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .map(|t| t.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                updated_at: DateTime::parse_from_rfc3339(&updated_str)
                    .map(|t| t.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                closed_at: closed_str.and_then(|s| {
                    DateTime::parse_from_rfc3339(&s)
                        .map(|t| t.with_timezone(&Utc))
                        .ok()
                }),
                resolution: Resolution::from_str(&resolution_str).unwrap_or(Resolution::None),
                close_reason: row.get(10)?,
            })
        })?;

        issues.collect::<std::result::Result<Vec<_>, _>>().map_err(StoreError::Database)
    }

    /// Adds a dependency between two issues.
    pub fn add_dependency(&self, issue_id: &str, depends_on_id: &str, dep_type: DepType) -> Result<()> {
        let dep = Dependency::new(issue_id, depends_on_id, dep_type);
        dep.validate().map_err(|e| StoreError::Validation(e.to_string()))?;

        self.conn.execute(
            r#"
            INSERT INTO dependencies (issue_id, depends_on_id, type, created_at)
            VALUES (?1, ?2, ?3, ?4)
            "#,
            params![
                dep.issue_id,
                dep.depends_on_id,
                dep.dep_type.as_str(),
                dep.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Removes a dependency.
    pub fn remove_dependency(&self, issue_id: &str, depends_on_id: &str, dep_type: DepType) -> Result<()> {
        self.conn.execute(
            r#"
            DELETE FROM dependencies WHERE issue_id = ?1 AND depends_on_id = ?2 AND type = ?3
            "#,
            params![issue_id, depends_on_id, dep_type.as_str()],
        )?;
        Ok(())
    }

    /// Removes all dependencies where the issue is the dependent.
    pub fn remove_all_dependencies(&self, issue_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM dependencies WHERE issue_id = ?1",
            params![issue_id],
        )?;
        Ok(())
    }

    /// Returns all dependencies for an issue.
    pub fn get_dependencies(&self, issue_id: &str) -> Result<Vec<Dependency>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT issue_id, depends_on_id, type, created_at
            FROM dependencies WHERE issue_id = ?1
            "#,
        )?;

        let deps = stmt.query_map(params![issue_id], |row| {
            let type_str: String = row.get(2)?;
            let created_str: String = row.get(3)?;

            Ok(Dependency {
                issue_id: row.get(0)?,
                depends_on_id: row.get(1)?,
                dep_type: DepType::from_str(&type_str).unwrap_or(DepType::Blocks),
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .map(|t| t.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
        })?;

        deps.collect::<std::result::Result<Vec<_>, _>>().map_err(StoreError::Database)
    }

    /// Returns all dependencies in the database, keyed by issue_id.
    pub fn get_all_dependencies(&self) -> Result<HashMap<String, Vec<Dependency>>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT issue_id, depends_on_id, type, created_at
            FROM dependencies
            "#,
        )?;

        let mut result: HashMap<String, Vec<Dependency>> = HashMap::new();

        let deps = stmt.query_map([], |row| {
            let type_str: String = row.get(2)?;
            let created_str: String = row.get(3)?;

            Ok(Dependency {
                issue_id: row.get(0)?,
                depends_on_id: row.get(1)?,
                dep_type: DepType::from_str(&type_str).unwrap_or(DepType::Blocks),
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .map(|t| t.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
        })?;

        for dep in deps {
            let d = dep?;
            result.entry(d.issue_id.clone()).or_default().push(d);
        }

        Ok(result)
    }

    /// Returns issues that are open and not blocked.
    pub fn get_ready_work(&self) -> Result<Vec<Issue>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT i.id, i.title, i.description, i.status, i.priority, i.issue_type,
                   i.created_at, i.updated_at, i.closed_at, COALESCE(i.resolution, '')
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
            "#,
        )?;

        let issues = stmt.query_map([], |row| {
            let status_str: String = row.get(3)?;
            let type_str: String = row.get(5)?;
            let created_str: String = row.get(6)?;
            let updated_str: String = row.get(7)?;
            let closed_str: Option<String> = row.get(8)?;
            let resolution_str: String = row.get(9)?;

            Ok(Issue {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                status: Status::from_str(&status_str).unwrap_or(Status::Open),
                priority: row.get(4)?,
                issue_type: IssueType::from_str(&type_str).unwrap_or(IssueType::Task),
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .map(|t| t.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                updated_at: DateTime::parse_from_rfc3339(&updated_str)
                    .map(|t| t.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                closed_at: closed_str.and_then(|s| {
                    DateTime::parse_from_rfc3339(&s)
                        .map(|t| t.with_timezone(&Utc))
                        .ok()
                }),
                resolution: Resolution::from_str(&resolution_str).unwrap_or(Resolution::None),
                close_reason: None,
            })
        })?;

        issues.collect::<std::result::Result<Vec<_>, _>>().map_err(StoreError::Database)
    }

    /// Returns issues that are blocked by the given issue ID.
    pub fn get_blocked_by(&self, blocker_id: &str) -> Result<Vec<Issue>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT i.id, i.title, i.description, i.status, i.priority, i.issue_type,
                   i.created_at, i.updated_at, i.closed_at, COALESCE(i.resolution, ''), i.close_reason
            FROM issues i
            JOIN dependencies d ON i.id = d.issue_id
            WHERE d.depends_on_id = ?1 AND d.type = 'blocks'
            ORDER BY i.priority ASC, i.created_at ASC
            "#,
        )?;

        let issues = stmt.query_map(params![blocker_id], |row| {
            let status_str: String = row.get(3)?;
            let type_str: String = row.get(5)?;
            let created_str: String = row.get(6)?;
            let updated_str: String = row.get(7)?;
            let closed_str: Option<String> = row.get(8)?;
            let resolution_str: String = row.get(9)?;

            Ok(Issue {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                status: Status::from_str(&status_str).unwrap_or(Status::Open),
                priority: row.get(4)?,
                issue_type: IssueType::from_str(&type_str).unwrap_or(IssueType::Task),
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .map(|t| t.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                updated_at: DateTime::parse_from_rfc3339(&updated_str)
                    .map(|t| t.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                closed_at: closed_str.and_then(|s| {
                    DateTime::parse_from_rfc3339(&s)
                        .map(|t| t.with_timezone(&Utc))
                        .ok()
                }),
                resolution: Resolution::from_str(&resolution_str).unwrap_or(Resolution::None),
                close_reason: row.get(10)?,
            })
        })?;

        issues.collect::<std::result::Result<Vec<_>, _>>().map_err(StoreError::Database)
    }

    /// Deletes an issue and all its dependencies.
    pub fn delete_issue(&self, id: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;

        tx.execute(
            "DELETE FROM dependencies WHERE issue_id = ?1 OR depends_on_id = ?1",
            params![id],
        )?;

        let rows = tx.execute("DELETE FROM issues WHERE id = ?1", params![id])?;
        if rows == 0 {
            return Err(StoreError::IssueNotFound);
        }

        tx.commit()?;
        Ok(())
    }

    /// Executes the given function within a database transaction.
    pub fn with_transaction<F, T>(&mut self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T>,
    {
        self.conn.execute("BEGIN IMMEDIATE", [])?;
        match f(&self.conn) {
            Ok(result) => {
                self.conn.execute("COMMIT", [])?;
                Ok(result)
            }
            Err(e) => {
                let _ = self.conn.execute("ROLLBACK", []);
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_get_issue() {
        let store = Store::in_memory().unwrap();
        let issue = Issue::new("Test task");
        store.create_issue(&issue).unwrap();

        let retrieved = store.get_issue(&issue.id).unwrap();
        assert_eq!(retrieved.title, "Test task");
        assert_eq!(retrieved.status, Status::Open);
    }

    #[test]
    fn test_list_issues() {
        let store = Store::in_memory().unwrap();
        store.create_issue(&Issue::new("Task 1")).unwrap();
        store.create_issue(&Issue::new("Task 2")).unwrap();

        let issues = store.list_issues().unwrap();
        assert_eq!(issues.len(), 2);
    }

    #[test]
    fn test_ready_work_with_blocking() {
        let store = Store::in_memory().unwrap();

        let issue_a = Issue::new("Task A");
        let issue_b = Issue::new("Task B");
        store.create_issue(&issue_a).unwrap();
        store.create_issue(&issue_b).unwrap();

        // B blocked by A
        store.add_dependency(&issue_b.id, &issue_a.id, DepType::Blocks).unwrap();

        let ready = store.get_ready_work().unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, issue_a.id);
    }

    #[test]
    fn test_close_unblocks_dependent() {
        let store = Store::in_memory().unwrap();

        let issue_a = Issue::new("Task A");
        let issue_b = Issue::new("Task B");
        store.create_issue(&issue_a).unwrap();
        store.create_issue(&issue_b).unwrap();
        store.add_dependency(&issue_b.id, &issue_a.id, DepType::Blocks).unwrap();

        // Close A
        store.close_issue(&issue_a.id, Resolution::Done, None).unwrap();

        // Now B should be ready
        let ready = store.get_ready_work().unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, issue_b.id);
    }

    #[test]
    fn test_delete_issue() {
        let store = Store::in_memory().unwrap();
        let issue = Issue::new("To delete");
        store.create_issue(&issue).unwrap();
        store.delete_issue(&issue.id).unwrap();

        assert!(matches!(store.get_issue(&issue.id), Err(StoreError::IssueNotFound)));
    }
}

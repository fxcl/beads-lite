//! Issue types and hash-based ID generation.

use chrono::{DateTime, Utc};
use num_bigint::BigUint;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use thiserror::Error;

/// Status represents the state of an issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Open,
    InProgress,
    Closed,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Open => "open",
            Status::InProgress => "in_progress",
            Status::Closed => "closed",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "open" => Some(Status::Open),
            "in_progress" => Some(Status::InProgress),
            "closed" => Some(Status::Closed),
            _ => None,
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// IssueType represents the category of an issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueType {
    Task,
    Bug,
    Feature,
    Epic,
}

impl IssueType {
    pub fn as_str(&self) -> &'static str {
        match self {
            IssueType::Task => "task",
            IssueType::Bug => "bug",
            IssueType::Feature => "feature",
            IssueType::Epic => "epic",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "task" => Some(IssueType::Task),
            "bug" => Some(IssueType::Bug),
            "feature" => Some(IssueType::Feature),
            "epic" => Some(IssueType::Epic),
            _ => None,
        }
    }
}

impl fmt::Display for IssueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Resolution represents why an issue was closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    #[default]
    #[serde(rename = "")]
    None,
    Done,
    Wontfix,
    Duplicate,
}

impl Resolution {
    pub fn as_str(&self) -> &'static str {
        match self {
            Resolution::None => "",
            Resolution::Done => "done",
            Resolution::Wontfix => "wontfix",
            Resolution::Duplicate => "duplicate",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "" => Some(Resolution::None),
            "done" => Some(Resolution::Done),
            "wontfix" => Some(Resolution::Wontfix),
            "duplicate" => Some(Resolution::Duplicate),
            _ => None,
        }
    }
}

impl fmt::Display for Resolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Error)]
pub enum IssueError {
    #[error("title cannot be empty")]
    EmptyTitle,
    #[error("invalid status: {0}")]
    InvalidStatus(String),
    #[error("invalid issue type: {0}")]
    InvalidType(String),
    #[error("priority must be 0-4, got {0}")]
    InvalidPriority(i32),
    #[error("invalid resolution: {0}")]
    InvalidResolution(String),
}

/// Issue represents a trackable work item with dependencies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
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
}

fn is_empty_resolution(r: &Resolution) -> bool {
    matches!(r, Resolution::None)
}

impl Issue {
    /// Creates a new issue with a hash-based ID and sensible defaults.
    pub fn new(title: impl Into<String>) -> Self {
        let title = title.into();
        let now = Utc::now();
        let id = generate_hash_id("bl", &title, "", now.timestamp_nanos_opt().unwrap_or(0), 4);

        Issue {
            id,
            title,
            description: String::new(),
            status: Status::Open,
            priority: 2,
            issue_type: IssueType::Task,
            created_at: now,
            updated_at: now,
            closed_at: None,
            resolution: Resolution::None,
            close_reason: None,
        }
    }

    /// Validates the issue fields.
    pub fn validate(&self) -> Result<(), IssueError> {
        if self.title.trim().is_empty() {
            return Err(IssueError::EmptyTitle);
        }
        if self.priority < 0 || self.priority > 4 {
            return Err(IssueError::InvalidPriority(self.priority));
        }
        Ok(())
    }
}

const BASE36_ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// Generates a hash-based ID for an issue.
fn generate_hash_id(prefix: &str, title: &str, description: &str, timestamp: i64, length: usize) -> String {
    let content = format!("{}|{}|{}", title, description, timestamp);
    let hash = Sha256::digest(content.as_bytes());

    let num_bytes = if length > 4 { 4 } else { 3 };
    let short_hash = encode_base36(&hash[..num_bytes], length);
    format!("{}-{}", prefix, short_hash)
}

/// Converts a byte slice to a base36 string of specified length.
fn encode_base36(data: &[u8], length: usize) -> String {
    let num = BigUint::from_bytes_be(data);
    let base = BigUint::from(36u32);
    let zero = BigUint::from(0u32);

    let mut chars = Vec::with_capacity(length);
    let mut n = num;

    while n > zero {
        let (quotient, remainder) = (&n / &base, &n % &base);
        let idx: usize = remainder.try_into().unwrap_or(0);
        chars.push(BASE36_ALPHABET[idx]);
        n = quotient;
    }

    chars.reverse();
    let mut result = String::from_utf8(chars).unwrap_or_default();

    // Pad with zeros if needed
    if result.len() < length {
        result = format!("{:0>width$}", result, width = length);
    }

    // Truncate to exact length if needed
    if result.len() > length {
        result = result[result.len() - length..].to_string();
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_issue() {
        let issue = Issue::new("Test task");
        assert!(issue.id.starts_with("bl-"));
        assert_eq!(issue.id.len(), 7); // bl-xxxx
        assert_eq!(issue.title, "Test task");
        assert_eq!(issue.status, Status::Open);
        assert_eq!(issue.priority, 2);
        assert_eq!(issue.issue_type, IssueType::Task);
    }

    #[test]
    fn test_validate_empty_title() {
        let mut issue = Issue::new("Test");
        issue.title = "   ".to_string();
        assert!(issue.validate().is_err());
    }

    #[test]
    fn test_validate_priority() {
        let mut issue = Issue::new("Test");
        issue.priority = 5;
        assert!(issue.validate().is_err());
        issue.priority = -1;
        assert!(issue.validate().is_err());
        issue.priority = 0;
        assert!(issue.validate().is_ok());
    }

    #[test]
    fn test_status_from_str() {
        assert_eq!(Status::from_str("open"), Some(Status::Open));
        assert_eq!(Status::from_str("in_progress"), Some(Status::InProgress));
        assert_eq!(Status::from_str("closed"), Some(Status::Closed));
        assert_eq!(Status::from_str("invalid"), None);
    }
}

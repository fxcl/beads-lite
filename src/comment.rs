//! Comment types and hash-based ID generation.
//!
//! Mirrors the [`crate::issue`] module's compact hash-based ID scheme but
//! uses a `cm-` prefix to keep comment IDs visually distinct from issue IDs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A comment attached to an issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub issue_id: String,
    pub author: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

impl Comment {
    /// Builds a new comment with a hash-based ID.
    pub fn new(issue_id: impl Into<String>, author: impl Into<String>, body: impl Into<String>) -> Self {
        let issue_id = issue_id.into();
        let author = author.into();
        let body = body.into();
        let now = Utc::now();
        let id = generate_comment_id(&issue_id, &author, &body, now.timestamp_nanos_opt().unwrap_or(0));
        Self {
            id,
            issue_id,
            author,
            body,
            created_at: now,
        }
    }

    /// Validates the comment fields.
    pub fn validate(&self) -> Result<(), CommentError> {
        if self.issue_id.trim().is_empty() {
            return Err(CommentError::EmptyIssueId);
        }
        if self.author.trim().is_empty() {
            return Err(CommentError::EmptyAuthor);
        }
        if self.body.trim().is_empty() {
            return Err(CommentError::EmptyBody);
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CommentError {
    #[error("comment issue_id must not be empty")]
    EmptyIssueId,
    #[error("comment author must not be empty")]
    EmptyAuthor,
    #[error("comment body must not be empty")]
    EmptyBody,
}

const BASE36_ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

fn generate_comment_id(issue_id: &str, author: &str, body: &str, timestamp: i64) -> String {
    use num_bigint::BigUint;
    use sha2::{Digest, Sha256};

    let content = format!("{}|{}|{}|{}", issue_id, author, body, timestamp);
    let hash = Sha256::digest(content.as_bytes());

    let num = BigUint::from_bytes_be(&hash[..3]);
    let base = BigUint::from(36u32);
    let zero = BigUint::from(0u32);

    let mut chars = Vec::with_capacity(4);
    let mut n = num;
    while n > zero {
        let (quotient, remainder) = (&n / &base, &n % &base);
        let idx: usize = remainder.try_into().unwrap_or(0);
        chars.push(BASE36_ALPHABET[idx]);
        n = quotient;
    }
    chars.reverse();
    let mut out = String::from_utf8(chars).unwrap_or_default();
    if out.len() < 4 {
        out = format!("{:0>4}", out);
    }
    if out.len() > 4 {
        out = out[out.len() - 4..].to_string();
    }
    format!("cm-{}", out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_comment_has_id() {
        let c = Comment::new("bl-abc1", "alice", "hello");
        assert!(c.id.starts_with("cm-"));
        assert_eq!(c.id.len(), 7); // cm-xxxx
        assert_eq!(c.author, "alice");
        assert_eq!(c.body, "hello");
    }

    #[test]
    fn validate_rejects_empty_fields() {
        assert!(Comment::new("bl-1", "alice", "").validate().is_err());
        assert!(Comment::new("bl-1", "", "x").validate().is_err());
        assert!(Comment::new("", "alice", "x").validate().is_err());
    }
}

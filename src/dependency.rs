//! Dependency types for issue relationships.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

/// DepType represents the type of dependency between issues.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepType {
    Blocks,
    DiscoveredFrom,
}

impl DepType {
    pub fn as_str(&self) -> &'static str {
        match self {
            DepType::Blocks => "blocks",
            DepType::DiscoveredFrom => "discovered_from",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "blocks" => Some(DepType::Blocks),
            "discovered_from" | "discovered-from" => Some(DepType::DiscoveredFrom),
            _ => None,
        }
    }
}

impl fmt::Display for DepType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Error)]
pub enum DependencyError {
    #[error("issue_id cannot be empty")]
    EmptyIssueId,
    #[error("depends_on_id cannot be empty")]
    EmptyDependsOnId,
    #[error("invalid dependency type: {0}")]
    InvalidType(String),
    #[error("issue cannot depend on itself")]
    SelfReference,
}

/// Dependency represents an edge in the issue dependency graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    pub issue_id: String,
    pub depends_on_id: String,
    #[serde(rename = "type")]
    pub dep_type: DepType,
    pub created_at: DateTime<Utc>,
}

impl Dependency {
    /// Creates a new dependency with the current timestamp.
    pub fn new(issue_id: impl Into<String>, depends_on_id: impl Into<String>, dep_type: DepType) -> Self {
        Dependency {
            issue_id: issue_id.into(),
            depends_on_id: depends_on_id.into(),
            dep_type,
            created_at: Utc::now(),
        }
    }

    /// Validates the dependency fields.
    pub fn validate(&self) -> Result<(), DependencyError> {
        if self.issue_id.is_empty() {
            return Err(DependencyError::EmptyIssueId);
        }
        if self.depends_on_id.is_empty() {
            return Err(DependencyError::EmptyDependsOnId);
        }
        if self.issue_id == self.depends_on_id {
            return Err(DependencyError::SelfReference);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_dependency() {
        let dep = Dependency::new("bl-0001", "bl-0002", DepType::Blocks);
        assert_eq!(dep.issue_id, "bl-0001");
        assert_eq!(dep.depends_on_id, "bl-0002");
        assert_eq!(dep.dep_type, DepType::Blocks);
    }

    #[test]
    fn test_validate_self_reference() {
        let dep = Dependency::new("bl-0001", "bl-0001", DepType::Blocks);
        assert!(dep.validate().is_err());
    }

    #[test]
    fn test_validate_empty_ids() {
        let dep = Dependency::new("", "bl-0002", DepType::Blocks);
        assert!(dep.validate().is_err());

        let dep = Dependency::new("bl-0001", "", DepType::Blocks);
        assert!(dep.validate().is_err());
    }

    #[test]
    fn test_discovered_from_type() {
        let dep = Dependency::new("bl-0001", "bl-0002", DepType::DiscoveredFrom);
        assert_eq!(dep.dep_type, DepType::DiscoveredFrom);
        assert_eq!(dep.dep_type.as_str(), "discovered_from");
    }

    #[test]
    fn test_dep_type_from_str() {
        assert_eq!(DepType::from_str("blocks"), Some(DepType::Blocks));
        assert_eq!(DepType::from_str("discovered_from"), Some(DepType::DiscoveredFrom));
        assert_eq!(DepType::from_str("discovered-from"), Some(DepType::DiscoveredFrom));
        assert_eq!(DepType::from_str("invalid"), None);
    }
}

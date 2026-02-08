//! Output formatting for list/ready commands.

use crate::dependency::Dependency;
use crate::issue::Issue;
use crate::jsonl::to_issue_export;
use crate::storage::Store;
use std::collections::HashMap;
use std::io::Write;

/// Formats a single issue line for table output.
pub fn format_issue_line(issue: &Issue) -> String {
    format!(
        "{}  {:<11}  P{}  {}  {}",
        issue.id, issue.status, issue.priority, issue.issue_type, issue.title
    )
}

/// Outputs issues in the appropriate format.
pub fn output_issues<W: Write>(store: &Store, issues: &[Issue], writer: &mut W, json_out: bool, tree_out: bool) -> std::io::Result<()> {
    if issues.is_empty() {
        if !json_out {
            writeln!(writer, "No issues found")?;
        }
        return Ok(());
    }

    if json_out {
        output_issues_json(store, issues, writer)?;
    } else if tree_out {
        output_issues_tree(store, issues, writer)?;
    } else {
        for issue in issues {
            writeln!(writer, "{}", format_issue_line(issue))?;
        }
    }
    Ok(())
}

/// Outputs issues as JSONL.
fn output_issues_json<W: Write>(store: &Store, issues: &[Issue], writer: &mut W) -> std::io::Result<()> {
    let all_deps = store.get_all_dependencies().unwrap_or_default();
    for issue in issues {
        let deps = all_deps.get(&issue.id).map(|v| v.as_slice()).unwrap_or(&[]);
        let export = to_issue_export(issue, deps);
        serde_json::to_writer(&mut *writer, &export).map_err(std::io::Error::other)?;
        writeln!(writer)?;
    }
    Ok(())
}

/// Outputs issues as a dependency tree.
fn output_issues_tree<W: Write>(store: &Store, issues: &[Issue], writer: &mut W) -> std::io::Result<()> {
    let all_deps = store.get_all_dependencies().unwrap_or_default();

    // Build issue map
    let issue_map: HashMap<&str, &Issue> = issues.iter().map(|i| (i.id.as_str(), i)).collect();

    // Identify children: issues that have an OPEN blocker in our list
    let mut children: HashMap<&str, Vec<&Issue>> = HashMap::new();
    let mut is_child: HashMap<&str, bool> = HashMap::new();

    for deps in all_deps.values() {
        for d in deps {
            if d.dep_type.as_str() != "blocks" {
                continue;
            }
            // d.issue_id is blocked by d.depends_on_id
            // So d.depends_on_id is the parent, d.issue_id is the child
            let child = issue_map.get(d.issue_id.as_str());
            let parent = issue_map.get(d.depends_on_id.as_str());

            if let (Some(child), Some(_)) = (child, parent) {
                children.entry(d.depends_on_id.as_str()).or_default().push(*child);
                is_child.insert(d.issue_id.as_str(), true);
            }
        }
    }

    // Roots are issues that aren't children of any open issue
    let mut roots: Vec<&Issue> = issues.iter().filter(|i| !is_child.contains_key(i.id.as_str())).collect();

    // Sort roots by priority then ID
    roots.sort_by(|a, b| a.priority.cmp(&b.priority).then_with(|| a.id.cmp(&b.id)));

    // Render tree
    for root in roots {
        writeln!(writer, "{}", format_issue_line(root))?;
        print_tree(writer, &children, &root.id, "")?;
    }

    Ok(())
}

/// Recursively prints children with tree-drawing characters.
fn print_tree<W: Write>(writer: &mut W, children: &HashMap<&str, Vec<&Issue>>, parent_id: &str, prefix: &str) -> std::io::Result<()> {
    let kids = children.get(parent_id);
    if kids.is_none() {
        return Ok(());
    }

    let mut kids: Vec<_> = kids.unwrap().clone();
    kids.sort_by(|a, b| a.priority.cmp(&b.priority).then_with(|| a.id.cmp(&b.id)));

    for (i, child) in kids.iter().enumerate() {
        let is_last = i == kids.len() - 1;
        let connector = if is_last { "└── " } else { "├── " };
        writeln!(writer, "{}{}{}", prefix, connector, format_issue_line(child))?;

        let extension = if is_last { "    " } else { "│   " };
        print_tree(writer, children, &child.id, &format!("{}{}", prefix, extension))?;
    }

    Ok(())
}

/// Outputs a single issue as JSON.
pub fn output_single_issue_json<W: Write>(issue: &Issue, deps: &[Dependency], writer: &mut W) -> std::io::Result<()> {
    let export = to_issue_export(issue, deps);
    serde_json::to_writer(&mut *writer, &export).map_err(std::io::Error::other)?;
    writeln!(writer)?;
    Ok(())
}

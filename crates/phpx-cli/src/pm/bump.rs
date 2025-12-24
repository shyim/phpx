//! Bump command - increase version constraints in composer.json to locked versions.

use anyhow::{Context, Result};
use clap::Args;
use console::style;
use regex::Regex;
use std::collections::HashMap;
use std::path::PathBuf;

use phpx_pm::json::{ComposerJson, ComposerLock};

#[derive(Args, Debug)]
pub struct BumpArgs {
    /// Only bump packages in require-dev
    #[arg(short = 'D', long)]
    pub dev_only: bool,

    /// Only bump packages in require (not require-dev)
    #[arg(short = 'R', long)]
    pub no_dev_only: bool,

    /// Show what would be changed without modifying files
    #[arg(long)]
    pub dry_run: bool,

    /// Working directory
    #[arg(short = 'd', long, default_value = ".")]
    pub working_dir: PathBuf,

    /// Specific packages to bump (optional, bumps all if not specified)
    #[arg(value_name = "PACKAGES")]
    pub packages: Vec<String>,
}

/// A version bump result
#[derive(Debug)]
struct BumpResult {
    package: String,
    old_constraint: String,
    new_constraint: String,
    is_dev: bool,
}

pub async fn execute(args: BumpArgs) -> Result<i32> {
    let working_dir = args.working_dir.canonicalize()
        .context("Failed to resolve working directory")?;

    // Load composer.json
    let json_path = working_dir.join("composer.json");
    if !json_path.exists() {
        eprintln!("{} No composer.json found in {}",
            style("Error:").red().bold(),
            working_dir.display()
        );
        return Ok(1);
    }

    let json_content = std::fs::read_to_string(&json_path)?;
    let composer_json: ComposerJson = serde_json::from_str(&json_content)
        .context("Failed to parse composer.json")?;

    // Load composer.lock
    let lock_path = working_dir.join("composer.lock");
    if !lock_path.exists() {
        eprintln!("{} No composer.lock found. Run 'phpx install' first.",
            style("Error:").red().bold()
        );
        return Ok(1);
    }

    let lock_content = std::fs::read_to_string(&lock_path)?;
    let composer_lock: ComposerLock = serde_json::from_str(&lock_content)
        .context("Failed to parse composer.lock")?;

    // Build a map of package name -> installed version from lock file
    let mut locked_versions: HashMap<String, String> = HashMap::new();
    for pkg in composer_lock.packages.iter().chain(composer_lock.packages_dev.iter()) {
        locked_versions.insert(pkg.name.to_lowercase(), pkg.version.clone());
    }

    // Collect bumps to make
    let mut bumps: Vec<BumpResult> = Vec::new();

    // Process require section
    if !args.dev_only {
        for (name, constraint) in &composer_json.require {
            if should_skip_package(name) {
                continue;
            }

            if !args.packages.is_empty() && !package_matches(&args.packages, name) {
                continue;
            }

            if let Some(installed_version) = locked_versions.get(&name.to_lowercase()) {
                if let Some(new_constraint) = bump_requirement(constraint, installed_version) {
                    if new_constraint != *constraint {
                        bumps.push(BumpResult {
                            package: name.clone(),
                            old_constraint: constraint.clone(),
                            new_constraint,
                            is_dev: false,
                        });
                    }
                }
            }
        }
    }

    // Process require-dev section
    if !args.no_dev_only {
        for (name, constraint) in &composer_json.require_dev {
            if should_skip_package(name) {
                continue;
            }

            if !args.packages.is_empty() && !package_matches(&args.packages, name) {
                continue;
            }

            if let Some(installed_version) = locked_versions.get(&name.to_lowercase()) {
                if let Some(new_constraint) = bump_requirement(constraint, installed_version) {
                    if new_constraint != *constraint {
                        bumps.push(BumpResult {
                            package: name.clone(),
                            old_constraint: constraint.clone(),
                            new_constraint,
                            is_dev: true,
                        });
                    }
                }
            }
        }
    }

    if bumps.is_empty() {
        println!("{} All version constraints are already up to date",
            style("Info:").cyan()
        );
        return Ok(0);
    }

    // Display changes
    if args.dry_run {
        println!("{} The following changes would be made:\n",
            style("Dry run:").yellow().bold()
        );
    } else {
        println!("{} Bumping version constraints:\n",
            style("Info:").cyan()
        );
    }

    for bump in &bumps {
        let section = if bump.is_dev { "require-dev" } else { "require" };
        println!("  {} ({}) {} -> {}",
            style(&bump.package).white().bold(),
            style(section).dim(),
            style(&bump.old_constraint).red(),
            style(&bump.new_constraint).green()
        );
    }

    if args.dry_run {
        println!("\n{} Run without --dry-run to apply changes",
            style("Note:").dim()
        );
        return Ok(0);
    }

    // Apply changes to composer.json
    // We need to preserve formatting, so we'll do a targeted replacement
    let mut updated_content = json_content.clone();

    for bump in &bumps {
        updated_content = update_constraint_in_json(
            &updated_content,
            &bump.package,
            &bump.old_constraint,
            &bump.new_constraint,
            bump.is_dev,
        )?;
    }

    // Write updated composer.json
    std::fs::write(&json_path, &updated_content)
        .context("Failed to write composer.json")?;

    println!("\n{} Updated {} constraint(s) in composer.json",
        style("Success:").green().bold(),
        bumps.len()
    );

    Ok(0)
}

/// Check if a package should be skipped (platform packages)
fn should_skip_package(name: &str) -> bool {
    name == "php" ||
    name.starts_with("ext-") ||
    name.starts_with("lib-") ||
    name == "composer-plugin-api" ||
    name == "composer-runtime-api"
}

/// Check if a package name matches any of the specified patterns
fn package_matches(patterns: &[String], name: &str) -> bool {
    let name_lower = name.to_lowercase();
    for pattern in patterns {
        let pattern_lower = pattern.to_lowercase();
        if pattern_lower.contains('*') {
            // Simple glob matching
            let regex_pattern = format!("^{}$",
                regex::escape(&pattern_lower).replace(r"\*", ".*")
            );
            if let Ok(re) = Regex::new(&regex_pattern) {
                if re.is_match(&name_lower) {
                    return true;
                }
            }
        } else if name_lower == pattern_lower {
            return true;
        }
    }
    false
}

/// Bump a version requirement based on installed version
/// Returns None if the constraint should not be bumped
fn bump_requirement(constraint: &str, installed_version: &str) -> Option<String> {
    let constraint = constraint.trim();
    let installed = normalize_version(installed_version);

    // Skip dev branches - can't be bumped
    if constraint.starts_with("dev-") || installed.starts_with("dev-") {
        return None;
    }

    // Skip if constraint contains branch alias
    if constraint.contains(" as ") {
        return None;
    }

    // Handle OR constraints (|) - bump each part separately
    if constraint.contains(" || ") || constraint.contains('|') {
        let parts: Vec<&str> = constraint.split(|c| c == '|')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        if parts.len() > 1 {
            let mut new_parts: Vec<String> = Vec::new();
            let mut any_changed = false;

            for part in parts {
                if let Some(bumped) = bump_single_constraint(part.trim(), &installed) {
                    if bumped != part.trim() {
                        any_changed = true;
                    }
                    new_parts.push(bumped);
                } else {
                    new_parts.push(part.trim().to_string());
                }
            }

            if any_changed {
                return Some(new_parts.join(" || "));
            }
            return Some(constraint.to_string());
        }
    }

    // Handle AND constraints (,) - these are ranges, bump the lower bound
    if constraint.contains(',') {
        return bump_range_constraint(constraint, &installed);
    }

    // Single constraint
    bump_single_constraint(constraint, &installed)
}

/// Bump a single constraint (no OR/AND)
fn bump_single_constraint(constraint: &str, installed: &str) -> Option<String> {
    let constraint = constraint.trim();

    // Parse constraint to extract operator and version
    let (op, version) = parse_constraint(constraint)?;

    match op {
        "^" => {
            // Caret: bump to installed version
            let installed_parts = parse_version_parts(installed);
            if installed_parts.is_empty() {
                return Some(constraint.to_string());
            }

            // Check if installed is greater than constraint version
            let constraint_parts = parse_version_parts(version);
            if !is_version_greater(&installed_parts, &constraint_parts) {
                return Some(constraint.to_string());
            }

            // Build new constraint, stripping trailing .0s
            let new_version = strip_trailing_zeros(&installed_parts);
            Some(format!("^{}", new_version))
        }
        "~" => {
            // Tilde: depends on specificity
            let installed_parts = parse_version_parts(installed);
            let constraint_parts = parse_version_parts(version);

            if !is_version_greater(&installed_parts, &constraint_parts) {
                return Some(constraint.to_string());
            }

            // If tilde has 2 parts (e.g., ~2.0), convert to caret
            // If tilde has 3+ parts (e.g., ~2.0.3), keep as tilde
            if constraint_parts.len() <= 2 {
                let new_version = strip_trailing_zeros(&installed_parts);
                Some(format!("^{}", new_version))
            } else {
                let new_version = format_version(&installed_parts, constraint_parts.len());
                Some(format!("~{}", new_version))
            }
        }
        ">=" => {
            // Greater-than-or-equal: bump to installed
            let installed_parts = parse_version_parts(installed);
            let constraint_parts = parse_version_parts(version);

            if !is_version_greater(&installed_parts, &constraint_parts) {
                return Some(constraint.to_string());
            }

            let new_version = strip_trailing_zeros(&installed_parts);
            Some(format!(">={}", new_version))
        }
        ">" => {
            // Greater-than: check if installed satisfies, then maybe bump
            Some(constraint.to_string())
        }
        "*" | "" => {
            // Wildcard: convert to caret with installed version
            let installed_parts = parse_version_parts(installed);
            if installed_parts.is_empty() {
                return Some(constraint.to_string());
            }
            let new_version = strip_trailing_zeros(&installed_parts);
            Some(format!("^{}", new_version))
        }
        "=" | "==" => {
            // Exact match: leave as is
            Some(constraint.to_string())
        }
        _ => {
            // Unknown operator, leave as is
            Some(constraint.to_string())
        }
    }
}

/// Bump a range constraint (e.g., ">=1.0 <2.0")
fn bump_range_constraint(constraint: &str, installed: &str) -> Option<String> {
    // Split by comma and/or space
    let parts: Vec<&str> = constraint.split(',')
        .flat_map(|s| s.split_whitespace())
        .filter(|s| !s.is_empty())
        .collect();

    if parts.is_empty() {
        return Some(constraint.to_string());
    }

    let mut new_parts: Vec<String> = Vec::new();
    let mut any_changed = false;

    for part in parts {
        let (op, _version) = parse_constraint(part)?;

        // Only bump lower bound constraints (>=, >)
        if op == ">=" || op == ">" {
            if let Some(bumped) = bump_single_constraint(part, installed) {
                if bumped != part {
                    any_changed = true;
                }
                new_parts.push(bumped);
            } else {
                new_parts.push(part.to_string());
            }
        } else {
            // Upper bounds (<, <=) stay the same
            new_parts.push(part.to_string());
        }
    }

    if any_changed {
        Some(new_parts.join(" "))
    } else {
        Some(constraint.to_string())
    }
}

/// Parse a constraint into (operator, version)
fn parse_constraint(constraint: &str) -> Option<(&str, &str)> {
    let constraint = constraint.trim();

    if constraint == "*" {
        return Some(("*", ""));
    }

    // Check for operators
    let ops = [">=", "<=", "!=", "==", "^", "~", ">", "<", "="];
    for op in ops {
        if constraint.starts_with(op) {
            let version = constraint[op.len()..].trim();
            return Some((op, version));
        }
    }

    // Check for wildcard version (e.g., "2.*")
    if constraint.contains('*') {
        return Some(("*", constraint));
    }

    // No operator means exact version or implicit ^
    if constraint.chars().next()?.is_ascii_digit() || constraint.starts_with('v') {
        return Some(("", constraint));
    }

    None
}

/// Parse version string into numeric parts
fn parse_version_parts(version: &str) -> Vec<u64> {
    let version = version.trim().trim_start_matches('v');

    // Remove stability suffix
    let version = if let Some(pos) = version.find('-') {
        &version[..pos]
    } else {
        version
    };

    version.split('.')
        .filter_map(|s| s.parse().ok())
        .collect()
}

/// Check if version a is greater than version b
fn is_version_greater(a: &[u64], b: &[u64]) -> bool {
    let max_len = std::cmp::max(a.len(), b.len());
    for i in 0..max_len {
        let a_val = a.get(i).copied().unwrap_or(0);
        let b_val = b.get(i).copied().unwrap_or(0);
        if a_val > b_val {
            return true;
        }
        if a_val < b_val {
            return false;
        }
    }
    false
}

/// Strip trailing zeros from version parts and format
fn strip_trailing_zeros(parts: &[u64]) -> String {
    let mut parts = parts.to_vec();

    // Always keep at least major.minor
    while parts.len() > 2 && parts.last() == Some(&0) {
        parts.pop();
    }

    parts.iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(".")
}

/// Format version with specific number of parts
fn format_version(parts: &[u64], num_parts: usize) -> String {
    let mut result: Vec<String> = parts.iter()
        .take(num_parts)
        .map(|n| n.to_string())
        .collect();

    // Pad with zeros if needed
    while result.len() < num_parts {
        result.push("0".to_string());
    }

    result.join(".")
}

/// Normalize version string
fn normalize_version(version: &str) -> String {
    let v = version.trim().trim_start_matches('v');
    // Remove stability suffix for comparison
    if let Some(pos) = v.find('-') {
        v[..pos].to_string()
    } else {
        v.to_string()
    }
}

/// Update a constraint in the raw JSON content
/// This preserves formatting better than re-serializing
fn update_constraint_in_json(
    content: &str,
    package: &str,
    old_constraint: &str,
    new_constraint: &str,
    is_dev: bool,
) -> Result<String> {
    let section = if is_dev { "require-dev" } else { "require" };

    // Build a regex to find the specific package in the section
    // This is a simple approach that works for standard composer.json formatting
    let pattern = format!(
        r#"("{}"[^}}]*?"{}"[^\n]*?:\s*)"{}""#,
        regex::escape(section),
        regex::escape(package),
        regex::escape(old_constraint)
    );

    if let Ok(re) = Regex::new(&pattern) {
        if re.is_match(content) {
            let replacement = format!(r#"$1"{}""#, new_constraint);
            return Ok(re.replace(content, replacement.as_str()).to_string());
        }
    }

    // Fallback: try simpler pattern just matching the package line
    let pattern2 = format!(
        r#"("{}"\s*:\s*)"{}""#,
        regex::escape(package),
        regex::escape(old_constraint)
    );

    if let Ok(re) = Regex::new(&pattern2) {
        if re.is_match(content) {
            let replacement = format!(r#"$1"{}""#, new_constraint);
            return Ok(re.replace(content, replacement.as_str()).to_string());
        }
    }

    // If regex fails, try a direct string replacement (less safe but works)
    let old_pattern = format!(r#""{}": "{}""#, package, old_constraint);
    let new_pattern = format!(r#""{}": "{}""#, package, new_constraint);

    if content.contains(&old_pattern) {
        return Ok(content.replace(&old_pattern, &new_pattern));
    }

    // Try with single quotes too (non-standard but possible)
    let old_pattern_sq = format!(r#"'{}': '{}'"#, package, old_constraint);
    let new_pattern_sq = format!(r#"'{}': '{}'"#, package, new_constraint);

    if content.contains(&old_pattern_sq) {
        return Ok(content.replace(&old_pattern_sq, &new_pattern_sq));
    }

    anyhow::bail!("Could not find package {} with constraint {} in JSON", package, old_constraint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bump_caret() {
        assert_eq!(bump_requirement("^1.0", "1.2.3"), Some("^1.2.3".to_string()));
        assert_eq!(bump_requirement("^1.0", "1.0.0"), Some("^1.0".to_string()));
        assert_eq!(bump_requirement("^2.0", "2.5.0"), Some("^2.5".to_string()));
    }

    #[test]
    fn test_bump_tilde() {
        assert_eq!(bump_requirement("~1.0", "1.2.3"), Some("^1.2.3".to_string()));
        assert_eq!(bump_requirement("~1.2.0", "1.2.5"), Some("~1.2.5".to_string()));
    }

    #[test]
    fn test_bump_gte() {
        assert_eq!(bump_requirement(">=1.0", "1.5.0"), Some(">=1.5".to_string()));
    }

    #[test]
    fn test_dev_branches() {
        assert_eq!(bump_requirement("dev-main", "dev-main"), None);
        assert_eq!(bump_requirement("dev-master", "dev-master"), None);
    }

    #[test]
    fn test_skip_platform() {
        assert!(should_skip_package("php"));
        assert!(should_skip_package("ext-json"));
        assert!(should_skip_package("lib-curl"));
        assert!(!should_skip_package("symfony/console"));
    }
}

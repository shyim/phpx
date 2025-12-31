//! Bump command - Increases the lower limit of composer.json requirements
//! to the currently installed versions.

use anyhow::{Context, Result};
use clap::Args;
use indexmap::IndexMap;
use regex::Regex;
use std::path::PathBuf;

use pox_pm::json::{ComposerJson, ComposerLock};
use pox_pm::{compute_content_hash, is_platform_package};

#[derive(Args, Debug)]
pub struct BumpArgs {
    /// Optional package name(s) to restrict which packages are bumped
    #[arg(value_name = "PACKAGES")]
    pub packages: Vec<String>,

    /// Only bump requirements in "require-dev"
    #[arg(short = 'D', long)]
    pub dev_only: bool,

    /// Only bump requirements in "require"
    #[arg(short = 'R', long)]
    pub no_dev_only: bool,

    /// Outputs the packages to bump, but will not execute anything
    #[arg(long)]
    pub dry_run: bool,

    /// Working directory
    #[arg(short = 'd', long, default_value = ".")]
    pub working_dir: PathBuf,
}

/// Bump a version constraint to match the installed version.
///
/// Examples:
/// - ^1.0 + 1.2.1 -> ^1.2.1
/// - ^1.2 + 1.2.0 -> ^1.2
/// - ~2.0 + 2.4.3 -> ^2.4.3
/// - ~2.2.3 + 2.2.6 -> ~2.2.6
/// - 2.* + 2.4.0 -> ^2.4
/// - >=3.0 + 3.4.5 -> >=3.4.5
/// - * + 1.2.3 -> >=1.2.3
/// - dev-main + dev-main -> dev-main (unchanged)
pub fn bump_requirement(constraint: &str, installed_version: &str) -> String {
    let constraint = constraint.trim();

    // Skip dev branch constraints
    if constraint.starts_with("dev-") {
        return constraint.to_string();
    }

    // Skip if installed version is dev (can't bump to dev)
    if installed_version.starts_with("dev-") {
        return constraint.to_string();
    }

    // Clean up installed version - remove dev suffix and trailing .0s or .9999999
    let version = clean_version(installed_version);

    // Skip non-stable versions (alpha, beta, RC, etc.)
    if !is_stable_version(&version) {
        return constraint.to_string();
    }

    // Get major version from installed version for pattern matching
    let major = get_major_version(&version);

    // Build the new constraint by replacing matching parts
    let new_constraint = bump_constraint_parts(constraint, &version, &major);

    // If the new constraint is equivalent to the old one, return the original
    if constraints_equivalent(constraint, &new_constraint) {
        return constraint.to_string();
    }

    new_constraint
}

/// Clean up a version string, removing dev suffix and unnecessary trailing parts
fn clean_version(version: &str) -> String {
    let version = version.trim();

    // Remove leading 'v' if present
    let version = version.strip_prefix('v').unwrap_or(version);
    let version = version.strip_prefix('V').unwrap_or(version);

    // Remove -dev suffix
    let version = version.strip_suffix("-dev").unwrap_or(version);

    // Remove trailing .0 or .9999999 parts
    let version = version
        .trim_end_matches(".0")
        .trim_end_matches(".9999999");

    // Remove any remaining -dev parts in the middle
    let version = if let Some(pos) = version.find("-dev") {
        &version[..pos]
    } else {
        version
    };

    // Remove stability suffixes like -alpha, -beta, -RC
    let version = if let Some(pos) = version.find("-alpha") {
        &version[..pos]
    } else {
        version
    };
    let version = if let Some(pos) = version.find("-beta") {
        &version[..pos]
    } else {
        version
    };
    let version = if let Some(pos) = version.find("-RC") {
        &version[..pos]
    } else {
        version
    };

    version.to_string()
}

/// Check if a version is stable (no alpha, beta, RC, dev suffixes)
fn is_stable_version(version: &str) -> bool {
    let lower = version.to_lowercase();
    !lower.contains("alpha")
        && !lower.contains("beta")
        && !lower.contains("-rc")
        && !lower.contains("dev")
        && !lower.contains("snapshot")
}

/// Get major version from a semver string
/// For "0.x" versions, returns "0.x" to handle pre-1.0 specially
fn get_major_version(version: &str) -> String {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.is_empty() {
        return version.to_string();
    }

    // For 0.x versions, include minor as well
    if parts[0] == "0" && parts.len() > 1 {
        format!("0\\.{}", regex::escape(parts[1]))
    } else {
        regex::escape(parts[0])
    }
}

/// Strip trailing .0s from a version, but keep at least major.minor
fn strip_trailing_zeros(version: &str) -> String {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() <= 2 {
        return version.to_string();
    }

    // Find the last non-zero part (but keep at least 2 parts)
    let mut keep = 2;
    for (i, part) in parts.iter().enumerate().skip(2) {
        if *part != "0" {
            keep = i + 1;
        }
    }

    parts[..keep].join(".")
}

/// Bump constraint parts, handling multi-constraints (||)
fn bump_constraint_parts(constraint: &str, version: &str, major: &str) -> String {
    // Handle multi-constraints separated by ||
    if constraint.contains("||") {
        let parts: Vec<&str> = constraint.split("||").collect();
        let bumped: Vec<String> = parts
            .into_iter()
            .map(|p| bump_single_constraint(p.trim(), version, major))
            .collect();
        return bumped.join(" || ");
    }

    bump_single_constraint(constraint, version, major)
}

/// Bump a single constraint (no || separators)
fn bump_single_constraint(constraint: &str, version: &str, major: &str) -> String {
    let constraint = constraint.trim();

    // Build pattern to match various constraint formats
    // This matches: ^major.*, ~major.*, major.*, >=major.*, *
    let pattern = format!(
        r"(?x)
        (?P<prefix>^|,|\s|\|)? # leading separator
        (?P<constraint>
            \^v?{major}(?:\.\d+)* # caret constraint like ^2.x.y
            | ~v?{major}(?:\.\d+){{1,3}} # tilde constraint like ~2.2 or ~2.2.2
            | v?{major}(?:\.[*x])+ # wildcard like 2.* or 2.x.x
            | >=v?\d+(?:\.\d+)* # greater-or-equal like >=2.0
            | \* # full wildcard
        )
        (?P<suffix>@\w+)? # stability suffix like @dev
        ",
        major = major
    );

    let re = match Regex::new(&pattern) {
        Ok(r) => r,
        Err(_) => return constraint.to_string(),
    };

    if !re.is_match(constraint) {
        return constraint.to_string();
    }

    let mut result = constraint.to_string();
    let mut offset: i64 = 0;

    // Find all matches and replace them from end to start to preserve offsets
    let matches: Vec<_> = re.captures_iter(constraint).collect();

    for caps in matches.iter().rev() {
        if let Some(m) = caps.name("constraint") {
            let old_constraint = m.as_str();
            let start = m.start() as i64 + offset;
            let end = m.end() as i64 + offset;

            let replacement = compute_replacement(old_constraint, version);
            let suffix = caps.name("suffix").map(|s| s.as_str()).unwrap_or("");

            let new_part = format!("{}{}", replacement, suffix);

            result = format!(
                "{}{}{}",
                &result[..(start as usize)],
                new_part,
                &result[(end as usize)..]
            );

            offset += new_part.len() as i64 - (end - start);
        }
    }

    result
}

/// Compute the replacement for a single constraint part
fn compute_replacement(old_constraint: &str, version: &str) -> String {
    let old = old_constraint.trim();

    // Count dots in old constraint to preserve precision level
    let old_dot_count = old.matches('.').count();
    let version_dot_count = version.matches('.').count();

    // Handle tilde constraints specially
    if old.starts_with('~') {
        // For ~X.Y.Z format (patch-level tilde), bump to ~X.Y.Z with same precision
        if old_dot_count >= 2 && !old.contains('*') && !old.contains('x') {
            // Take as many version bits as we have in the constraint
            let version_parts: Vec<&str> = version.split('.').collect();
            let mut result_parts = version_parts.clone();

            // Pad with zeros if needed
            while result_parts.len() <= old_dot_count {
                result_parts.push("0");
            }

            // Take only as many parts as in original constraint
            let result: Vec<&str> = result_parts.into_iter().take(old_dot_count + 1).collect();
            return format!("~{}", result.join("."));
        }
        // For ~X or ~X.Y, convert to caret
        let clean_version = strip_trailing_zeros(version);
        return format!("^{}", clean_version);
    }

    // Handle caret constraints
    if old.starts_with('^') {
        // Preserve precision: if old was ^X.Y.Z, keep 3 parts
        if old_dot_count >= 2 {
            let version_parts: Vec<&str> = version.split('.').collect();
            let mut result_parts = version_parts.clone();

            // Pad with zeros if needed
            while result_parts.len() <= old_dot_count {
                result_parts.push("0");
            }

            // Take only as many parts as in original
            let result: Vec<&str> = result_parts.into_iter().take(old_dot_count + 1).collect();
            return format!("^{}", result.join("."));
        }

        // Otherwise strip trailing zeros
        let clean_version = strip_trailing_zeros(version);
        return format!("^{}", clean_version);
    }

    // Handle wildcard constraints like 2.* or 2.x
    if old.contains('*') || old.contains('x') || old.contains('X') {
        // Convert to caret
        let clean_version = strip_trailing_zeros(version);
        return format!("^{}", clean_version);
    }

    // Handle >= constraints
    if old.starts_with(">=") {
        // Preserve precision
        if old_dot_count >= 2 || version_dot_count >= 2 {
            let version_parts: Vec<&str> = version.split('.').collect();
            let mut result_parts = version_parts.clone();

            // Pad with zeros if needed
            while result_parts.len() <= old_dot_count.max(1) {
                result_parts.push("0");
            }

            let result: Vec<&str> = result_parts
                .into_iter()
                .take((old_dot_count + 1).max(2))
                .collect();
            return format!(">={}", result.join("."));
        }

        let clean_version = strip_trailing_zeros(version);
        return format!(">={}", clean_version);
    }

    // Handle full wildcard *
    if old == "*" {
        let clean_version = strip_trailing_zeros(version);
        return format!(">={}", clean_version);
    }

    // Default: return caret constraint
    let clean_version = strip_trailing_zeros(version);
    format!("^{}", clean_version)
}

/// Check if two constraints are functionally equivalent
fn constraints_equivalent(old: &str, new: &str) -> bool {
    // Simple string comparison for now
    // A more sophisticated implementation would parse and compare semver ranges
    let old_normalized = old.replace('v', "").replace('V', "");
    let new_normalized = new.replace('v', "").replace('V', "");
    old_normalized == new_normalized
}

/// Updates for require and require-dev
pub struct BumpUpdates {
    pub require: IndexMap<String, String>,
    pub require_dev: IndexMap<String, String>,
}

/// Calculate which packages need bumping
pub fn calculate_updates(
    composer_json: &ComposerJson,
    lock: &ComposerLock,
    packages_filter: &[String],
    dev_only: bool,
    no_dev_only: bool,
) -> BumpUpdates {
    let mut updates = BumpUpdates {
        require: IndexMap::new(),
        require_dev: IndexMap::new(),
    };

    // Build filter pattern if packages specified
    let filter_patterns: Vec<Regex> = packages_filter
        .iter()
        .filter_map(|p| {
            // Strip version constraint if present (e.g., "pkg:^1.0" -> "pkg")
            let name = p.split(':').next().unwrap_or(p);
            let pattern = name
                .replace('*', ".*")
                .replace('?', ".");
            Regex::new(&format!("^{}$", pattern)).ok()
        })
        .collect();

    let matches_filter = |name: &str| -> bool {
        if filter_patterns.is_empty() {
            return true;
        }
        let name_lower = name.to_lowercase();
        filter_patterns.iter().any(|p| p.is_match(&name_lower))
    };

    // Process require packages
    if !dev_only {
        for (name, constraint) in &composer_json.require {
            if is_platform_package(name) {
                continue;
            }
            if !matches_filter(name) {
                continue;
            }

            if let Some(pkg) = lock.find_package(name) {
                let bumped = bump_requirement(constraint, &pkg.version);
                if bumped != *constraint {
                    updates.require.insert(name.clone(), bumped);
                }
            }
        }
    }

    // Process require-dev packages
    if !no_dev_only {
        for (name, constraint) in &composer_json.require_dev {
            if is_platform_package(name) {
                continue;
            }
            if !matches_filter(name) {
                continue;
            }

            if let Some(pkg) = lock.find_package(name) {
                let bumped = bump_requirement(constraint, &pkg.version);
                if bumped != *constraint {
                    updates.require_dev.insert(name.clone(), bumped);
                }
            }
        }
    }

    updates
}

/// Apply updates to composer.json content, preserving formatting
pub fn apply_updates_to_json(content: &str, updates: &BumpUpdates) -> Result<String> {
    let mut result = content.to_string();

    // Apply require updates
    for (name, new_version) in &updates.require {
        result = update_dependency_in_json(&result, "require", name, new_version)?;
    }

    // Apply require-dev updates
    for (name, new_version) in &updates.require_dev {
        result = update_dependency_in_json(&result, "require-dev", name, new_version)?;
    }

    Ok(result)
}

/// Update a single dependency in JSON content
fn update_dependency_in_json(
    content: &str,
    section: &str,
    name: &str,
    new_version: &str,
) -> Result<String> {
    // Build a pattern to find the dependency line
    // This handles: "package/name": "^1.0"
    let escaped_name = regex::escape(name);
    let pattern = format!(
        r#"("{}")\s*:\s*"([^"]*)""#,
        escaped_name
    );

    let re = Regex::new(&pattern).context("Failed to build regex pattern")?;

    // Find the section first
    let section_pattern = format!(r#""{}"\s*:\s*\{{"#, regex::escape(section));
    let section_re = Regex::new(&section_pattern)?;

    if let Some(section_match) = section_re.find(content) {
        let section_start = section_match.start();

        // Find the closing brace for this section by counting braces
        let remaining = &content[section_start..];
        let mut brace_count = 0;
        let mut section_end = remaining.len();

        for (i, ch) in remaining.chars().enumerate() {
            match ch {
                '{' => brace_count += 1,
                '}' => {
                    brace_count -= 1;
                    if brace_count == 0 {
                        section_end = i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }

        let section_content = &content[section_start..section_start + section_end];

        // Now find and replace the dependency in this section
        if let Some(caps) = re.captures(section_content) {
            let full_match = caps.get(0).unwrap();
            let replacement = format!(r#"{}": "{}""#, &caps[1], new_version);

            let new_section = format!(
                "{}{}{}",
                &section_content[..full_match.start()],
                replacement,
                &section_content[full_match.end()..]
            );

            return Ok(format!(
                "{}{}{}",
                &content[..section_start],
                new_section,
                &content[section_start + section_end..]
            ));
        }
    }

    // If we couldn't find it with the smart approach, try a simpler global replacement
    // (less safe but works as fallback)
    if let Some(caps) = re.captures(content) {
        let full_match = caps.get(0).unwrap();
        let replacement = format!(r#"{}": "{}""#, &caps[1], new_version);

        return Ok(format!(
            "{}{}{}",
            &content[..full_match.start()],
            replacement,
            &content[full_match.end()..]
        ));
    }

    Ok(content.to_string())
}

pub async fn execute(args: BumpArgs) -> Result<i32> {
    let working_dir = args
        .working_dir
        .canonicalize()
        .context("Failed to resolve working directory")?;

    let json_path = working_dir.join("composer.json");
    let lock_path = working_dir.join("composer.lock");

    // Check if composer.json exists and is readable
    if !json_path.exists() {
        eprintln!("./composer.json is not readable.");
        return Ok(1);
    }

    // Read composer.json content (for both parsing and later modification)
    let json_content = std::fs::read_to_string(&json_path)
        .context("Failed to read composer.json")?;

    // Parse composer.json
    let composer_json: ComposerJson = serde_json::from_str(&json_content)
        .context("Failed to parse composer.json")?;

    // Check if this is a library (non-project type)
    if composer_json.package_type != "project" && !args.dev_only {
        eprintln!("Warning: Bumping dependency constraints is not recommended for libraries as it will narrow down your dependencies and may cause problems for your users.");
        if composer_json.package_type == "library" {
            eprintln!("If your package is not a library, you can explicitly specify the \"type\" by using \"composer config type project\".");
            eprintln!("Alternatively you can use --dev-only to only bump dependencies within \"require-dev\".");
        }
    }

    // Read lock file if available
    let lock: ComposerLock = if lock_path.exists() {
        let lock_content = std::fs::read_to_string(&lock_path)
            .context("Failed to read composer.lock")?;
        serde_json::from_str(&lock_content)
            .context("Failed to parse composer.lock")?
    } else {
        // Try to read from installed.json if no lock file
        let installed_path = working_dir.join("vendor/composer/installed.json");
        if installed_path.exists() {
            // Read installed.json and convert to lock-like structure
            let installed_content = std::fs::read_to_string(&installed_path)
                .context("Failed to read installed.json")?;
            parse_installed_json(&installed_content)?
        } else {
            eprintln!("No composer.lock or vendor/composer/installed.json found.");
            eprintln!("Run 'pox install' first to create a lock file.");
            return Ok(1);
        }
    };

    // Calculate updates
    let updates = calculate_updates(
        &composer_json,
        &lock,
        &args.packages,
        args.dev_only,
        args.no_dev_only,
    );

    let change_count = updates.require.len() + updates.require_dev.len();

    if change_count > 0 {
        if args.dry_run {
            println!("./composer.json would be updated with:");
            for (name, version) in &updates.require {
                println!("  - require.{}: {}", name, version);
            }
            for (name, version) in &updates.require_dev {
                println!("  - require-dev.{}: {}", name, version);
            }
            // Return 1 in dry-run mode when there are changes (matches composer behavior)
            return Ok(1);
        }

        // Apply updates to the JSON content
        let new_content = apply_updates_to_json(&json_content, &updates)?;

        // Check if file is writable
        let metadata = std::fs::metadata(&json_path)?;
        if metadata.permissions().readonly() {
            eprintln!("./composer.json is not writable.");
            return Ok(1);
        }

        // Write the updated content
        std::fs::write(&json_path, &new_content)
            .context("Failed to write composer.json")?;

        println!(
            "./composer.json has been updated ({} changes).",
            change_count
        );

        // Update lock file hash if lock file exists
        if lock_path.exists() {
            update_lock_hash(&lock_path, &new_content)?;
        }
    } else {
        println!("No requirements to update in ./composer.json.");
    }

    Ok(0)
}

/// Parse installed.json into a ComposerLock-like structure
fn parse_installed_json(content: &str) -> Result<ComposerLock> {
    use pox_pm::json::LockedPackage;

    #[derive(serde::Deserialize)]
    struct InstalledJson {
        packages: Option<Vec<LockedPackage>>,
        #[serde(rename = "dev-package-names")]
        dev_package_names: Option<Vec<String>>,
    }

    // Try parsing as the new format first
    if let Ok(installed) = serde_json::from_str::<InstalledJson>(content) {
        let all_packages = installed.packages.unwrap_or_default();
        let dev_names: std::collections::HashSet<String> = installed
            .dev_package_names
            .unwrap_or_default()
            .into_iter()
            .map(|n| n.to_lowercase())
            .collect();

        let (dev_packages, packages): (Vec<_>, Vec<_>) = all_packages
            .into_iter()
            .partition(|p| dev_names.contains(&p.name.to_lowercase()));

        return Ok(ComposerLock {
            packages,
            packages_dev: dev_packages,
            ..Default::default()
        });
    }

    // Try old format (direct array)
    if let Ok(packages) = serde_json::from_str::<Vec<LockedPackage>>(content) {
        return Ok(ComposerLock {
            packages,
            ..Default::default()
        });
    }

    anyhow::bail!("Failed to parse installed.json")
}

/// Update the content-hash in composer.lock after modifying composer.json
fn update_lock_hash(lock_path: &std::path::Path, json_content: &str) -> Result<()> {
    let lock_content = std::fs::read_to_string(lock_path)?;

    // Compute new hash
    let new_hash = compute_content_hash(json_content);

    // Update the hash in lock content
    let hash_pattern = Regex::new(r#""content-hash"\s*:\s*"[a-f0-9]+""#)?;
    let new_lock_content = hash_pattern
        .replace(&lock_content, format!(r#""content-hash": "{}""#, new_hash))
        .to_string();

    std::fs::write(lock_path, new_lock_content)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bump_caret_basic() {
        assert_eq!(bump_requirement("^1.0", "1.2.1"), "^1.2.1");
        assert_eq!(bump_requirement("^1.0", "1.2.0"), "^1.2");
        assert_eq!(bump_requirement("^1.2", "1.2.0"), "^1.2");
    }

    #[test]
    fn test_bump_caret_preserves_precision() {
        assert_eq!(bump_requirement("^1.0.0", "1.2.0"), "^1.2.0");
        assert_eq!(bump_requirement("^1.0.0", "1.2.1"), "^1.2.1");
    }

    #[test]
    fn test_bump_tilde() {
        assert_eq!(bump_requirement("~2.0", "2.4.3"), "^2.4.3");
        assert_eq!(bump_requirement("~2.2.3", "2.2.6"), "~2.2.6");
        assert_eq!(bump_requirement("~2.2.3", "2.2.6.2"), "~2.2.6");
    }

    #[test]
    fn test_bump_wildcard() {
        assert_eq!(bump_requirement("2.*", "2.4.0"), "^2.4");
        assert_eq!(bump_requirement("2.x", "2.4.0"), "^2.4");
        assert_eq!(bump_requirement("2.x.x", "2.4.0"), "^2.4.0");
    }

    #[test]
    fn test_bump_greater_or_equal() {
        assert_eq!(bump_requirement(">=3.0", "3.4.5"), ">=3.4.5");
        assert_eq!(bump_requirement(">=1.0", "1.2.3"), ">=1.2.3");
    }

    #[test]
    fn test_bump_full_wildcard() {
        assert_eq!(bump_requirement("*", "1.2.3"), ">=1.2.3");
    }

    #[test]
    fn test_bump_multi_constraint() {
        assert_eq!(bump_requirement("^1.2 || ^2.3", "1.3.2"), "^1.3.2 || ^2.3");
        assert_eq!(bump_requirement("^1.2 || ^2.3", "2.4.0"), "^1.2 || ^2.4");
    }

    #[test]
    fn test_bump_skip_dev() {
        assert_eq!(bump_requirement("dev-main", "dev-foo"), "dev-main");
        assert_eq!(bump_requirement("^3.2", "dev-main"), "^3.2");
    }

    #[test]
    fn test_bump_skip_unstable() {
        assert_eq!(bump_requirement("~2", "2.1-beta.1"), "~2");
    }

    #[test]
    fn test_clean_version() {
        assert_eq!(clean_version("1.2.3"), "1.2.3");
        assert_eq!(clean_version("v1.2.3"), "1.2.3");
        assert_eq!(clean_version("1.2.0"), "1.2");
        assert_eq!(clean_version("1.0.0"), "1");
        assert_eq!(clean_version("1.2.3-dev"), "1.2.3");
        assert_eq!(clean_version("1.2.3.9999999-dev"), "1.2.3");
    }

    #[test]
    fn test_strip_trailing_zeros() {
        assert_eq!(strip_trailing_zeros("1.2.3"), "1.2.3");
        assert_eq!(strip_trailing_zeros("1.2.0"), "1.2");
        assert_eq!(strip_trailing_zeros("1.0.0"), "1.0");
        assert_eq!(strip_trailing_zeros("1.0"), "1.0");
    }
}

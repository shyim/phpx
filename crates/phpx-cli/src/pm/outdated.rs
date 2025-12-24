//! Outdated command - show packages with newer versions available.

use anyhow::{Context, Result};
use clap::Args;
use console::style;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use phpx_pm::config::AuthConfig;
use phpx_pm::json::{ComposerJson, ComposerLock};
use phpx_pm::repository::{ComposerRepository, Repository};

#[derive(Args, Debug)]
pub struct OutdatedArgs {
    /// Package name to check (optional, checks all if not specified)
    #[arg(value_name = "PACKAGE")]
    pub package: Option<String>,

    /// Only show direct dependencies
    #[arg(short = 'D', long)]
    pub direct: bool,

    /// Only show packages with minor version updates
    #[arg(long)]
    pub minor_only: bool,

    /// Only show packages with major version updates
    #[arg(long)]
    pub major_only: bool,

    /// Only show packages with patch version updates
    #[arg(long)]
    pub patch_only: bool,

    /// Ignore specific packages (comma-separated)
    #[arg(long)]
    pub ignore: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub format_json: bool,

    /// Working directory
    #[arg(short = 'd', long, default_value = ".")]
    pub working_dir: PathBuf,
}

/// Information about an outdated package
#[derive(Debug, Clone, serde::Serialize)]
pub struct OutdatedPackage {
    pub name: String,
    pub current_version: String,
    pub latest_version: String,
    pub description: Option<String>,
    pub is_direct: bool,
    pub update_type: UpdateType,
    pub abandoned: Option<String>,
}

/// Type of version update
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateType {
    Major,
    Minor,
    Patch,
}

impl std::fmt::Display for UpdateType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateType::Major => write!(f, "major"),
            UpdateType::Minor => write!(f, "minor"),
            UpdateType::Patch => write!(f, "patch"),
        }
    }
}

pub async fn execute(args: OutdatedArgs) -> Result<i32> {
    let working_dir = args.working_dir.canonicalize()
        .context("Failed to resolve working directory")?;

    // Load composer.json
    let json_path = working_dir.join("composer.json");
    let composer_json: Option<ComposerJson> = if json_path.exists() {
        let content = std::fs::read_to_string(&json_path)?;
        Some(serde_json::from_str(&content)?)
    } else {
        None
    };

    // Load composer.lock
    let lock_path = working_dir.join("composer.lock");
    if !lock_path.exists() {
        eprintln!("{} No composer.lock found. Run 'phpx install' first.",
            style("Error:").red().bold()
        );
        return Ok(1);
    }

    let lock_content = std::fs::read_to_string(&lock_path)?;
    let composer_lock: ComposerLock = serde_json::from_str(&lock_content)?;

    // Build set of direct dependencies
    let direct_deps: HashSet<String> = composer_json
        .as_ref()
        .map(|json| {
            json.require.keys()
                .chain(json.require_dev.keys())
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    // Build set of ignored packages
    let ignored: HashSet<String> = args.ignore
        .as_ref()
        .map(|s| s.split(',').map(|p| p.trim().to_lowercase()).collect())
        .unwrap_or_default();

    // Load authentication for private repositories
    let auth = AuthConfig::build(Some(&working_dir)).unwrap_or_default();

    // Create repository for querying latest versions
    let mut repo = ComposerRepository::packagist();
    repo.set_auth(auth);

    // Collect packages to check
    let packages_to_check: Vec<_> = composer_lock.packages.iter()
        .chain(composer_lock.packages_dev.iter())
        .filter(|p| {
            // Filter by specific package if provided
            if let Some(ref name) = args.package {
                if !p.name.eq_ignore_ascii_case(name) {
                    return false;
                }
            }
            // Filter ignored packages
            if ignored.contains(&p.name.to_lowercase()) {
                return false;
            }
            // Filter by direct dependencies if requested
            if args.direct && !direct_deps.contains(&p.name) {
                return false;
            }
            // Skip platform packages
            if p.name.starts_with("php") || p.name.starts_with("ext-") || p.name.starts_with("lib-") {
                return false;
            }
            true
        })
        .collect();

    if packages_to_check.is_empty() {
        if args.package.is_some() {
            eprintln!("{} Package not found or is ignored",
                style("Error:").red().bold()
            );
            return Ok(1);
        }
        println!("{} No packages to check", style("Info:").cyan());
        return Ok(0);
    }

    println!("{} Checking {} packages for updates...",
        style("Info:").cyan(),
        packages_to_check.len()
    );

    // Check each package for updates
    let mut outdated: Vec<OutdatedPackage> = Vec::new();

    for pkg in packages_to_check {
        // Query repository for available versions
        let available = repo.find_packages(&pkg.name).await;

        if available.is_empty() {
            continue;
        }

        // Find the latest stable version
        let latest = find_latest_stable_version(&available);

        if let Some(latest_pkg) = latest {
            let current = normalize_version(&pkg.version);
            let latest_ver = normalize_version(&latest_pkg.version);

            // Compare versions
            if let Some(update_type) = compare_versions(&current, &latest_ver) {
                // Apply filters
                if args.major_only && update_type != UpdateType::Major {
                    continue;
                }
                if args.minor_only && update_type != UpdateType::Minor {
                    continue;
                }
                if args.patch_only && update_type != UpdateType::Patch {
                    continue;
                }

                let abandoned = latest_pkg.abandoned.as_ref()
                    .and_then(|a| a.replacement().map(|s| s.to_string()));

                outdated.push(OutdatedPackage {
                    name: pkg.name.clone(),
                    current_version: pkg.version.clone(),
                    latest_version: latest_pkg.version.clone(),
                    description: latest_pkg.description.clone(),
                    is_direct: direct_deps.contains(&pkg.name),
                    update_type,
                    abandoned,
                });
            }
        }
    }

    // Sort by name
    outdated.sort_by(|a, b| a.name.cmp(&b.name));

    // Output
    if outdated.is_empty() {
        println!("\n{} All packages are up to date!", style("Success:").green().bold());
        return Ok(0);
    }

    if args.format_json {
        println!("{}", serde_json::to_string_pretty(&outdated)?);
        return Ok(0);
    }

    // Calculate column widths
    let max_name_len = outdated.iter()
        .map(|p| p.name.len())
        .max()
        .unwrap_or(20)
        .min(50);

    let max_current_len = outdated.iter()
        .map(|p| p.current_version.len())
        .max()
        .unwrap_or(10);

    let max_latest_len = outdated.iter()
        .map(|p| p.latest_version.len())
        .max()
        .unwrap_or(10);

    // Print header
    println!();
    println!("{} {} outdated package(s):\n",
        style("Found").yellow().bold(),
        outdated.len()
    );

    // Color legend for update types
    let color_major = style("!").red().bold();
    let color_minor = style("~").yellow();
    let color_patch = style(".").green();

    for pkg in &outdated {
        let marker = match pkg.update_type {
            UpdateType::Major => color_major.clone(),
            UpdateType::Minor => color_minor.clone(),
            UpdateType::Patch => color_patch.clone(),
        };

        let direct_marker = if pkg.is_direct { style("*").cyan() } else { style(" ") };

        let version_arrow = format!("{} -> {}",
            style(&pkg.current_version).dim(),
            match pkg.update_type {
                UpdateType::Major => style(&pkg.latest_version).red().bold(),
                UpdateType::Minor => style(&pkg.latest_version).yellow(),
                UpdateType::Patch => style(&pkg.latest_version).green(),
            }
        );

        println!("{}{} {:<width_name$} {:<width_ver$}",
            marker,
            direct_marker,
            style(&pkg.name).white().bold(),
            version_arrow,
            width_name = max_name_len,
            width_ver = max_current_len + max_latest_len + 4,
        );

        // Show abandonment warning
        if let Some(ref replacement) = pkg.abandoned {
            println!("    {} Package is abandoned. Use {} instead.",
                style("Warning:").yellow(),
                style(replacement).cyan()
            );
        }
    }

    // Print legend
    println!();
    println!("{}", style("Legend:").dim());
    println!("  {} = major update (breaking changes)", style("!").red().bold());
    println!("  {} = minor update (new features)", style("~").yellow());
    println!("  {} = patch update (bug fixes)", style(".").green());
    println!("  {} = direct dependency", style("*").cyan());

    Ok(0)
}

/// Find the latest stable version from a list of packages
fn find_latest_stable_version(packages: &[Arc<phpx_pm::package::Package>]) -> Option<Arc<phpx_pm::package::Package>> {
    // Filter to stable versions only
    let mut stable_versions: Vec<_> = packages.iter()
        .filter(|p| {
            let v = p.version.to_lowercase();
            // Skip dev/alpha/beta/RC versions unless no stable exists
            !v.contains("-dev") &&
            !v.contains("alpha") &&
            !v.contains("beta") &&
            !v.contains("-rc") &&
            !v.starts_with("dev-")
        })
        .cloned()
        .collect();

    // Sort by version (descending)
    stable_versions.sort_by(|a, b| {
        compare_version_strings(&b.version, &a.version)
    });

    stable_versions.into_iter().next()
}

/// Normalize version string for comparison
fn normalize_version(version: &str) -> String {
    let v = version.trim_start_matches('v');
    // Remove stability suffix
    if let Some(pos) = v.find('-') {
        v[..pos].to_string()
    } else {
        v.to_string()
    }
}

/// Compare two version strings and return update type if newer
fn compare_versions(current: &str, latest: &str) -> Option<UpdateType> {
    let current_parts: Vec<u64> = current.split('.')
        .filter_map(|s| s.parse().ok())
        .collect();
    let latest_parts: Vec<u64> = latest.split('.')
        .filter_map(|s| s.parse().ok())
        .collect();

    if latest_parts.is_empty() || current_parts.is_empty() {
        return None;
    }

    let current_major = current_parts.first().copied().unwrap_or(0);
    let current_minor = current_parts.get(1).copied().unwrap_or(0);
    let current_patch = current_parts.get(2).copied().unwrap_or(0);

    let latest_major = latest_parts.first().copied().unwrap_or(0);
    let latest_minor = latest_parts.get(1).copied().unwrap_or(0);
    let latest_patch = latest_parts.get(2).copied().unwrap_or(0);

    if latest_major > current_major {
        Some(UpdateType::Major)
    } else if latest_major == current_major && latest_minor > current_minor {
        Some(UpdateType::Minor)
    } else if latest_major == current_major && latest_minor == current_minor && latest_patch > current_patch {
        Some(UpdateType::Patch)
    } else {
        None
    }
}

/// Compare two version strings for sorting
fn compare_version_strings(a: &str, b: &str) -> std::cmp::Ordering {
    let a_parts: Vec<u64> = normalize_version(a).split('.')
        .filter_map(|s| s.parse().ok())
        .collect();
    let b_parts: Vec<u64> = normalize_version(b).split('.')
        .filter_map(|s| s.parse().ok())
        .collect();

    for i in 0..std::cmp::max(a_parts.len(), b_parts.len()) {
        let a_val = a_parts.get(i).copied().unwrap_or(0);
        let b_val = b_parts.get(i).copied().unwrap_or(0);

        match a_val.cmp(&b_val) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }

    std::cmp::Ordering::Equal
}

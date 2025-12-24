//! Why command - show why a package is installed (reverse dependency lookup).

use anyhow::{Context, Result};
use clap::Args;
use console::style;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use phpx_pm::json::{ComposerJson, ComposerLock, LockedPackage};

#[derive(Args, Debug)]
pub struct WhyArgs {
    /// Package name to check
    #[arg(value_name = "PACKAGE")]
    pub package: String,

    /// Show recursive dependencies (full dependency chain)
    #[arg(short = 'r', long)]
    pub recursive: bool,

    /// Show as tree
    #[arg(short = 't', long)]
    pub tree: bool,

    /// Output as JSON
    #[arg(long)]
    pub format_json: bool,

    /// Working directory
    #[arg(short = 'd', long, default_value = ".")]
    pub working_dir: PathBuf,
}

/// Represents a dependency relationship
#[derive(Debug, Clone, serde::Serialize)]
pub struct DependencyReason {
    /// Package that depends on the target
    pub package: String,
    /// Version of the depending package
    pub version: String,
    /// Version constraint required
    pub constraint: String,
    /// Whether it's a dev dependency
    pub is_dev: bool,
}

pub async fn execute(args: WhyArgs) -> Result<i32> {
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

    // Check if the package exists
    let target_package = args.package.to_lowercase();
    let package_exists = composer_lock.packages.iter()
        .chain(composer_lock.packages_dev.iter())
        .any(|p| p.name.to_lowercase() == target_package);

    if !package_exists {
        eprintln!("{} Package '{}' is not installed",
            style("Error:").red().bold(),
            args.package
        );
        return Ok(1);
    }

    // Build reverse dependency map
    let reverse_deps = build_reverse_dependency_map(&composer_lock);

    // Find direct dependents
    let mut reasons: Vec<DependencyReason> = Vec::new();

    // Check root composer.json
    if let Some(ref json) = composer_json {
        if let Some(constraint) = json.require.get(&args.package) {
            reasons.push(DependencyReason {
                package: "__root__".to_string(),
                version: "".to_string(),
                constraint: constraint.clone(),
                is_dev: false,
            });
        }
        if let Some(constraint) = json.require_dev.get(&args.package) {
            reasons.push(DependencyReason {
                package: "__root__".to_string(),
                version: "".to_string(),
                constraint: constraint.clone(),
                is_dev: true,
            });
        }
    }

    // Check installed packages
    if let Some(dependents) = reverse_deps.get(&target_package) {
        for (pkg_name, constraint, is_dev) in dependents {
            let pkg_version = composer_lock.packages.iter()
                .chain(composer_lock.packages_dev.iter())
                .find(|p| &p.name == pkg_name)
                .map(|p| p.version.clone())
                .unwrap_or_default();

            reasons.push(DependencyReason {
                package: pkg_name.clone(),
                version: pkg_version,
                constraint: constraint.clone(),
                is_dev: *is_dev,
            });
        }
    }

    if reasons.is_empty() {
        println!("{} Package '{}' is not required by any other package",
            style("Info:").cyan(),
            args.package
        );
        return Ok(0);
    }

    // Output
    if args.format_json {
        println!("{}", serde_json::to_string_pretty(&reasons)?);
        return Ok(0);
    }

    if args.tree && args.recursive {
        // Show full dependency tree
        print_dependency_tree(&args.package, &composer_json, &composer_lock, &reverse_deps, 0, &mut HashSet::new());
    } else {
        // Show direct dependents
        println!("{} is required by:\n", style(&args.package).white().bold());

        for reason in &reasons {
            let pkg_display = if reason.package == "__root__" {
                style("Root composer.json").cyan().to_string()
            } else {
                format!("{} {}",
                    style(&reason.package).white().bold(),
                    style(&reason.version).yellow()
                )
            };

            let dep_type = if reason.is_dev {
                style("(dev)").dim()
            } else {
                style("")
            };

            println!("  {} {} {}",
                pkg_display,
                style(&reason.constraint).green(),
                dep_type
            );
        }

        if args.recursive && !args.tree {
            println!();
            println!("{}", style("Full dependency chain:").dim());

            // Show chain for each non-root dependent
            for reason in &reasons {
                if reason.package != "__root__" {
                    print_dependency_chain(&reason.package, &composer_json, &composer_lock, &reverse_deps, 1, &mut HashSet::new());
                }
            }
        }
    }

    Ok(0)
}

/// Build a map of package -> list of (dependent_package, constraint, is_dev)
fn build_reverse_dependency_map(lock: &ComposerLock) -> HashMap<String, Vec<(String, String, bool)>> {
    let mut reverse_deps: HashMap<String, Vec<(String, String, bool)>> = HashMap::new();

    for pkg in &lock.packages {
        add_package_deps(&mut reverse_deps, pkg, false);
    }

    for pkg in &lock.packages_dev {
        add_package_deps(&mut reverse_deps, pkg, true);
    }

    reverse_deps
}

fn add_package_deps(
    reverse_deps: &mut HashMap<String, Vec<(String, String, bool)>>,
    pkg: &LockedPackage,
    is_dev_pkg: bool,
) {
    for (dep_name, constraint) in &pkg.require {
        let key = dep_name.to_lowercase();
        reverse_deps
            .entry(key)
            .or_default()
            .push((pkg.name.clone(), constraint.clone(), false));
    }

    for (dep_name, constraint) in &pkg.require_dev {
        let key = dep_name.to_lowercase();
        reverse_deps
            .entry(key)
            .or_default()
            .push((pkg.name.clone(), constraint.clone(), true || is_dev_pkg));
    }
}

fn print_dependency_tree(
    package: &str,
    composer_json: &Option<ComposerJson>,
    lock: &ComposerLock,
    reverse_deps: &HashMap<String, Vec<(String, String, bool)>>,
    depth: usize,
    visited: &mut HashSet<String>,
) {
    let indent = "  ".repeat(depth);
    let pkg_lower = package.to_lowercase();

    // Prevent infinite loops
    if visited.contains(&pkg_lower) {
        println!("{}{}  {}", indent, style(package).dim(), style("(circular)").red());
        return;
    }
    visited.insert(pkg_lower.clone());

    // Print current package
    if depth == 0 {
        println!("{}", style(package).white().bold());
    }

    // Check root
    if let Some(ref json) = composer_json {
        if let Some(constraint) = json.require.get(package) {
            println!("{}{}  {} {}", indent, style("__root__").cyan(), style(constraint).green(), "");
        }
        if let Some(constraint) = json.require_dev.get(package) {
            println!("{}{}  {} {}", indent, style("__root__").cyan(), style(constraint).green(), style("(dev)").dim());
        }
    }

    // Check dependents
    if let Some(dependents) = reverse_deps.get(&pkg_lower) {
        for (pkg_name, constraint, is_dev) in dependents {
            let pkg_version = lock.packages.iter()
                .chain(lock.packages_dev.iter())
                .find(|p| &p.name == pkg_name)
                .map(|p| p.version.as_str())
                .unwrap_or("");

            let dev_marker = if *is_dev { style(" (dev)").dim() } else { style("") };

            println!("{}{} {}  {}{}",
                indent,
                style(pkg_name).white(),
                style(pkg_version).yellow(),
                style(constraint).green(),
                dev_marker
            );

            // Recurse
            print_dependency_tree(pkg_name, composer_json, lock, reverse_deps, depth + 1, visited);
        }
    }

    visited.remove(&pkg_lower);
}

fn print_dependency_chain(
    package: &str,
    composer_json: &Option<ComposerJson>,
    lock: &ComposerLock,
    reverse_deps: &HashMap<String, Vec<(String, String, bool)>>,
    depth: usize,
    visited: &mut HashSet<String>,
) {
    let indent = "  ".repeat(depth);
    let pkg_lower = package.to_lowercase();

    if visited.contains(&pkg_lower) {
        return;
    }
    visited.insert(pkg_lower.clone());

    // Check if this package is in root
    let is_root_dep = composer_json.as_ref().map(|json| {
        json.require.contains_key(package) || json.require_dev.contains_key(package)
    }).unwrap_or(false);

    if is_root_dep {
        println!("{}{} -> __root__", indent, style(package).white());
    }

    // Find dependents of this package
    if let Some(dependents) = reverse_deps.get(&pkg_lower) {
        for (pkg_name, _constraint, _is_dev) in dependents {
            println!("{}{} -> {}", indent, style(package).white(), style(pkg_name).cyan());
            print_dependency_chain(pkg_name, composer_json, lock, reverse_deps, depth + 1, visited);
        }
    }

    visited.remove(&pkg_lower);
}

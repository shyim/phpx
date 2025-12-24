//! Show command - display information about packages.

use anyhow::{Context, Result};
use clap::Args;
use console::style;
use std::path::PathBuf;

use phpx_pm::json::{ComposerJson, ComposerLock};

#[derive(Args, Debug)]
pub struct ShowArgs {
    /// Package name to show details for
    #[arg(value_name = "PACKAGE")]
    pub package: Option<String>,

    /// List installed packages only
    #[arg(short = 'i', long)]
    pub installed: bool,

    /// List platform packages only
    #[arg(short = 'p', long)]
    pub platform: bool,

    /// Show available packages (from repositories)
    #[arg(short = 'a', long)]
    pub available: bool,

    /// Only show direct dependencies
    #[arg(short = 'D', long)]
    pub direct: bool,

    /// Show dependency tree
    #[arg(short = 't', long)]
    pub tree: bool,

    /// Show outdated packages
    #[arg(short = 'o', long)]
    pub outdated: bool,

    /// Output as JSON
    #[arg(long)]
    pub format_json: bool,

    /// Working directory
    #[arg(short = 'd', long, default_value = ".")]
    pub working_dir: PathBuf,
}

pub async fn execute(args: ShowArgs) -> Result<i32> {
    let working_dir = args.working_dir.canonicalize()
        .context("Failed to resolve working directory")?;

    let json_path = working_dir.join("composer.json");
    let composer_json: Option<ComposerJson> = if json_path.exists() {
        let content = std::fs::read_to_string(&json_path)?;
        Some(serde_json::from_str(&content)?)
    } else {
        None
    };

    let lock_path = working_dir.join("composer.lock");
    let composer_lock: Option<ComposerLock> = if lock_path.exists() {
        let content = std::fs::read_to_string(&lock_path)?;
        Some(serde_json::from_str(&content)?)
    } else {
        None
    };

    if let Some(ref name) = args.package {
        return show_package_details(name, &composer_json, &composer_lock, args.format_json);
    }

    if args.installed || composer_lock.is_some() {
        return show_installed_packages(&composer_json, &composer_lock, &args);
    }

    println!("{} No composer.lock found. Run 'phpx composer install' first.",
        style("Info:").cyan()
    );

    Ok(0)
}

fn show_package_details(
    name: &str,
    _composer_json: &Option<ComposerJson>,
    composer_lock: &Option<ComposerLock>,
    as_json: bool,
) -> Result<i32> {
    let package = composer_lock.as_ref()
        .and_then(|lock| {
            lock.packages.iter()
                .chain(lock.packages_dev.iter())
                .find(|p| p.name == name)
        });

    let Some(pkg) = package else {
        eprintln!("{} Package '{}' not found",
            style("Error:").red().bold(),
            name
        );
        return Ok(1);
    };

    if as_json {
        println!("{}", serde_json::to_string_pretty(pkg)?);
        return Ok(0);
    }

    println!("{} {}", style("name").cyan(), style(&pkg.name).white().bold());
    println!("{} {}", style("version").cyan(), style(&pkg.version).yellow());

    if let Some(desc) = &pkg.description {
        println!("{} {}", style("description").cyan(), desc);
    }

    if let Some(homepage) = &pkg.homepage {
        println!("{} {}", style("homepage").cyan(), homepage);
    }

    if !pkg.license.is_empty() {
        println!("{} {}", style("license").cyan(), pkg.license.join(", "));
    }

    if !pkg.keywords.is_empty() {
        println!("{} {}", style("keywords").cyan(), pkg.keywords.join(", "));
    }

    if !pkg.authors.is_empty() {
        println!("{}", style("authors").cyan());
        for author in &pkg.authors {
            let name = &author.name;
            if let Some(email) = &author.email {
                println!("  {} <{}>", name, email);
            } else {
                println!("  {}", name);
            }
        }
    }

    if !pkg.require.is_empty() {
        println!("{}", style("requires").cyan());
        for (dep, constraint) in &pkg.require {
            println!("  {} {}", dep, style(constraint).dim());
        }
    }

    Ok(0)
}

fn show_installed_packages(
    composer_json: &Option<ComposerJson>,
    composer_lock: &Option<ComposerLock>,
    args: &ShowArgs,
) -> Result<i32> {
    let Some(lock) = composer_lock else {
        println!("{} No packages installed", style("Info:").cyan());
        return Ok(0);
    };

    let direct_deps: std::collections::HashSet<String> = composer_json
        .as_ref()
        .map(|json| {
            json.require.keys()
                .chain(json.require_dev.keys())
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    if args.format_json {
        let packages: Vec<_> = lock.packages.iter()
            .chain(lock.packages_dev.iter())
            .filter(|p| !args.direct || direct_deps.contains(&p.name))
            .collect();
        println!("{}", serde_json::to_string_pretty(&packages)?);
        return Ok(0);
    }

    let all_packages: Vec<_> = lock.packages.iter()
        .chain(lock.packages_dev.iter())
        .filter(|p| !args.direct || direct_deps.contains(&p.name))
        .collect();

    if all_packages.is_empty() {
        println!("{} No packages installed", style("Info:").cyan());
        return Ok(0);
    }

    let max_name_len = all_packages.iter()
        .map(|p| p.name.len())
        .max()
        .unwrap_or(20);

    let max_version_len = all_packages.iter()
        .map(|p| p.version.len())
        .max()
        .unwrap_or(10);

    for pkg in all_packages {
        let is_direct = direct_deps.contains(&pkg.name);
        let marker = if is_direct { style("*").green() } else { style(" ").dim() };

        println!("{} {:<width_name$} {:<width_ver$} {}",
            marker,
            style(&pkg.name).white().bold(),
            style(&pkg.version).yellow(),
            pkg.description.as_deref().unwrap_or(""),
            width_name = max_name_len,
            width_ver = max_version_len,
        );
    }

    println!();
    println!("{} {} packages installed ({} direct)",
        style("Legend:").dim(),
        lock.packages.len() + lock.packages_dev.len(),
        direct_deps.len()
    );
    println!("  {} = direct dependency", style("*").green());

    Ok(0)
}

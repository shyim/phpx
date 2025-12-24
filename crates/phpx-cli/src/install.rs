//! Install command - install project dependencies.

use anyhow::{Context, Result};
use clap::Args;
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use phpx_pm::{
    autoload::{AutoloadConfig, AutoloadGenerator, PackageAutoload},
    http::HttpClient,
    installer::{InstallConfig, InstallationManager},
    json::{ComposerJson, ComposerLock},
    Package,
    package::{Autoload, AutoloadPath, Dist, Source},
};

use crate::pm::scripts;

#[derive(Args, Debug)]
pub struct InstallArgs {
    /// Prefer source installation (git clone)
    #[arg(long)]
    pub prefer_source: bool,

    /// Prefer dist installation (zip download)
    #[arg(long)]
    pub prefer_dist: bool,

    /// Run in dry-run mode (no actual changes)
    #[arg(long)]
    pub dry_run: bool,

    /// Skip dev dependencies
    #[arg(long)]
    pub no_dev: bool,

    /// Skip autoloader generation
    #[arg(long)]
    pub no_autoloader: bool,

    /// Skip script execution
    #[arg(long)]
    pub no_scripts: bool,

    /// Disable progress output
    #[arg(long)]
    pub no_progress: bool,

    /// Optimize autoloader (convert PSR-4/PSR-0 to classmap)
    #[arg(short = 'o', long)]
    pub optimize_autoloader: bool,

    /// Use authoritative classmap (only load from classmap)
    #[arg(short = 'a', long)]
    pub classmap_authoritative: bool,

    /// Use APCu to cache found/not-found classes
    #[arg(long)]
    pub apcu_autoloader: bool,

    /// Ignore platform requirements
    #[arg(long)]
    pub ignore_platform_reqs: bool,

    /// Working directory
    #[arg(short = 'd', long, default_value = ".")]
    pub working_dir: PathBuf,
}

pub async fn execute(args: InstallArgs) -> Result<i32> {
    let working_dir = args.working_dir.canonicalize()
        .context("Failed to resolve working directory")?;

    // Load composer.json for scripts
    let json_path = working_dir.join("composer.json");
    let composer_json: Option<ComposerJson> = if json_path.exists() {
        let content = std::fs::read_to_string(&json_path)?;
        Some(serde_json::from_str(&content)?)
    } else {
        None
    };

    // Check for composer.lock
    let lock_path = working_dir.join("composer.lock");
    if !lock_path.exists() {
        eprintln!("{} No composer.lock file found.", style("Error:").red().bold());
        eprintln!("Run 'phpx update' to generate one.");
        return Ok(1);
    }

    // Parse composer.lock
    let lock_content = std::fs::read_to_string(&lock_path)
        .context("Failed to read composer.lock")?;
    let lock: ComposerLock = serde_json::from_str(&lock_content)
        .context("Failed to parse composer.lock")?;

    // Run pre-install-cmd script
    if !args.no_scripts {
        if let Some(ref json) = composer_json {
            let exit_code = scripts::run_event_script("pre-install-cmd", json, &working_dir, false)?;
            if exit_code != 0 {
                return Ok(exit_code);
            }
        }
    }

    // Convert locked packages to Package structs
    let mut packages: Vec<Package> = lock.packages.iter()
        .map(locked_package_to_package)
        .collect();

    if !args.no_dev {
        packages.extend(lock.packages_dev.iter().map(locked_package_to_package));
    }

    if packages.is_empty() {
        println!("{} Nothing to install.", style("Info:").cyan());
        return Ok(0);
    }

    println!("{} Installing dependencies from lock file", style("Composer").green().bold());

    if args.dry_run {
        println!("{} Running in dry-run mode", style("Info:").cyan());
    }

    // Create progress bar
    let progress = if args.no_progress {
        ProgressBar::hidden()
    } else {
        let pb = ProgressBar::new(packages.len() as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg}")
                .unwrap()
                .progress_chars("#>-"),
        );
        pb.enable_steady_tick(Duration::from_millis(100));
        pb
    };

    // Setup installation
    let http_client = Arc::new(HttpClient::new()
        .context("Failed to create HTTP client")?);

    let install_config = InstallConfig {
        vendor_dir: working_dir.join("vendor"),
        bin_dir: working_dir.join("vendor/bin"),
        cache_dir: dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".phpx/cache")),
        prefer_source: args.prefer_source,
        prefer_dist: args.prefer_dist || !args.prefer_source,
        dry_run: args.dry_run,
        no_dev: args.no_dev,
    };

    let manager = InstallationManager::new(http_client.clone(), install_config.clone());

    // Install packages
    let result = manager.install_packages(&packages).await
        .context("Failed to install packages")?;

    progress.finish_and_clear();

    // Report results
    if !result.installed.is_empty() {
        for pkg in &result.installed {
            println!("  {} {} ({})",
                style("-").green(),
                style(&pkg.name).white().bold(),
                style(&pkg.version).yellow()
            );
        }
    }

    // Generate autoloader
    if !args.no_autoloader && !args.dry_run {
        // Run pre-autoload-dump script
        if !args.no_scripts {
            if let Some(ref json) = composer_json {
                let exit_code = scripts::run_event_script("pre-autoload-dump", json, &working_dir, false)?;
                if exit_code != 0 {
                    return Ok(exit_code);
                }
            }
        }

        println!("{} Generating autoload files", style("Info:").cyan());

        // Convert packages to PackageAutoload
        let mut package_autoloads: Vec<PackageAutoload> = lock.packages.iter()
            .map(locked_package_to_autoload)
            .collect();
        if !args.no_dev {
            package_autoloads.extend(lock.packages_dev.iter().map(locked_package_to_autoload));
        }

        let autoload_config = AutoloadConfig {
            vendor_dir: install_config.vendor_dir.clone(),
            base_dir: working_dir.clone(),
            optimize: args.optimize_autoloader || args.classmap_authoritative,
            apcu: args.apcu_autoloader,
            authoritative: args.classmap_authoritative,
            suffix: if !lock.content_hash.is_empty() {
                Some(lock.content_hash.clone())
            } else {
                None
            },
        };

        let generator = AutoloadGenerator::new(autoload_config);

        // Get root autoload from composer.json
        let root_autoload: Option<Autoload> = composer_json.as_ref()
            .and_then(|_| {
                // Re-read to get the raw autoload value
                let content = std::fs::read_to_string(&json_path).ok()?;
                let raw: serde_json::Value = serde_json::from_str(&content).ok()?;
                raw.get("autoload")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
            });

        generator.generate(&package_autoloads, root_autoload.as_ref())
            .context("Failed to generate autoloader")?;

        // Run post-autoload-dump script
        if !args.no_scripts {
            if let Some(ref json) = composer_json {
                let exit_code = scripts::run_event_script("post-autoload-dump", json, &working_dir, false)?;
                if exit_code != 0 {
                    return Ok(exit_code);
                }
            }
        }
    }

    println!("{} {} packages installed",
        style("Success:").green().bold(),
        result.installed.len()
    );

    // Run post-install-cmd script
    if !args.no_scripts && !args.dry_run {
        if let Some(ref json) = composer_json {
            let exit_code = scripts::run_event_script("post-install-cmd", json, &working_dir, false)?;
            if exit_code != 0 {
                return Ok(exit_code);
            }
        }
    }

    Ok(0)
}

/// Convert a LockedPackage to a Package
fn locked_package_to_package(lp: &phpx_pm::json::LockedPackage) -> Package {
    let mut pkg = Package::new(&lp.name, &lp.version);

    pkg.description = lp.description.clone();
    pkg.homepage = lp.homepage.clone();
    pkg.license = lp.license.clone();
    pkg.keywords = lp.keywords.clone();
    pkg.require = lp.require.clone();
    pkg.require_dev = lp.require_dev.clone();
    pkg.conflict = lp.conflict.clone();
    pkg.provide = lp.provide.clone();
    pkg.replace = lp.replace.clone();
    pkg.suggest = lp.suggest.clone();
    pkg.bin = lp.bin.clone();
    pkg.package_type = lp.package_type.clone();

    // Convert source
    if let Some(src) = &lp.source {
        pkg.source = Some(Source::new(&src.source_type, &src.url, &src.reference));
    }

    // Convert dist
    if let Some(dist) = &lp.dist {
        let mut d = Dist::new(&dist.dist_type, &dist.url);
        if let Some(ref r) = dist.reference {
            d = d.with_reference(r);
        }
        if let Some(ref s) = dist.shasum {
            d = d.with_shasum(s);
        }
        pkg.dist = Some(d);
    }

    pkg
}

/// Convert a LockedPackage to a PackageAutoload
fn locked_package_to_autoload(lp: &phpx_pm::json::LockedPackage) -> PackageAutoload {
    let autoload = convert_lock_autoload(&lp.autoload);

    // Extract requires (filter out platform requirements like php, ext-*)
    let requires: Vec<String> = lp.require.keys()
        .filter(|k| *k != "php" && !k.starts_with("ext-") && !k.starts_with("lib-"))
        .cloned()
        .collect();

    PackageAutoload {
        name: lp.name.clone(),
        autoload,
        install_path: lp.name.clone(),
        requires,
    }
}

/// Convert LockAutoload to Autoload
fn convert_lock_autoload(lock_autoload: &phpx_pm::json::LockAutoload) -> Autoload {
    let mut autoload = Autoload::default();

    // Convert PSR-4
    for (namespace, value) in &lock_autoload.psr4 {
        let paths = json_value_to_paths(value);
        autoload.psr4.insert(namespace.clone(), paths);
    }

    // Convert PSR-0
    for (namespace, value) in &lock_autoload.psr0 {
        let paths = json_value_to_paths(value);
        autoload.psr0.insert(namespace.clone(), paths);
    }

    // Classmap
    autoload.classmap = lock_autoload.classmap.clone();

    // Files
    autoload.files = lock_autoload.files.clone();

    // Exclude from classmap
    autoload.exclude_from_classmap = lock_autoload.exclude_from_classmap.clone();

    autoload
}

/// Convert JSON value to AutoloadPath
fn json_value_to_paths(value: &serde_json::Value) -> AutoloadPath {
    match value {
        serde_json::Value::String(s) => AutoloadPath::Single(s.clone()),
        serde_json::Value::Array(arr) => {
            let paths: Vec<String> = arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            if paths.len() == 1 {
                AutoloadPath::Single(paths[0].clone())
            } else {
                AutoloadPath::Multiple(paths)
            }
        }
        _ => AutoloadPath::Single(String::new()),
    }
}

/// Get the cache directory
mod dirs {
    use std::path::PathBuf;

    pub fn cache_dir() -> Option<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Caches/phpx"))
        }

        #[cfg(target_os = "linux")]
        {
            std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
                .map(|p| p.join("phpx"))
        }

        #[cfg(target_os = "windows")]
        {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .map(|p| p.join("phpx"))
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".phpx/cache"))
        }
    }
}

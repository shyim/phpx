//! Dump-autoload command - regenerate the autoloader.

use anyhow::{Context, Result};
use clap::Args;
use console::style;
use std::path::PathBuf;

use phpx_pm::{
    autoload::{AutoloadConfig, AutoloadGenerator, PackageAutoload},
    json::ComposerLock,
    package::Autoload,
};

#[derive(Args, Debug)]
pub struct DumpAutoloadArgs {
    /// Optimize autoloader (convert PSR-4/PSR-0 to classmap)
    #[arg(short = 'o', long)]
    pub optimize: bool,

    /// Use authoritative classmap (only load from classmap)
    #[arg(short = 'a', long)]
    pub classmap_authoritative: bool,

    /// Use APCu to cache found/not-found classes
    #[arg(long)]
    pub apcu: bool,

    /// Skip dev dependencies
    #[arg(long)]
    pub no_dev: bool,

    /// Working directory
    #[arg(short = 'd', long, default_value = ".")]
    pub working_dir: PathBuf,
}

pub async fn execute(args: DumpAutoloadArgs) -> Result<i32> {
    let working_dir = args.working_dir.canonicalize()
        .context("Failed to resolve working directory")?;

    let vendor_dir = working_dir.join("vendor");

    if !vendor_dir.exists() {
        eprintln!("{} vendor directory not found. Run 'phpx composer install' first.",
            style("Error:").red().bold()
        );
        return Ok(1);
    }

    println!("{} Generating autoload files", style("Composer").green().bold());

    // Load composer.lock to get packages and content-hash for suffix
    let lock_path = working_dir.join("composer.lock");
    let (packages, suffix) = if lock_path.exists() {
        let content = std::fs::read_to_string(&lock_path)
            .context("Failed to read composer.lock")?;
        let lock: ComposerLock = serde_json::from_str(&content)
            .context("Failed to parse composer.lock")?;

        let mut pkgs: Vec<PackageAutoload> = lock.packages.iter()
            .map(locked_package_to_autoload)
            .collect();

        if !args.no_dev {
            pkgs.extend(lock.packages_dev.iter().map(locked_package_to_autoload));
        }

        // Use the content-hash as the suffix
        let suffix = if !lock.content_hash.is_empty() {
            Some(lock.content_hash.clone())
        } else {
            None
        };

        (pkgs, suffix)
    } else {
        (Vec::new(), None)
    };

    // Get root autoload from composer.json
    let composer_json_path = working_dir.join("composer.json");
    let root_autoload: Option<Autoload> = if composer_json_path.exists() {
        let content = std::fs::read_to_string(&composer_json_path)?;
        let json: serde_json::Value = serde_json::from_str(&content)?;
        json.get("autoload")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    } else {
        None
    };

    // Generate autoloader
    let config = AutoloadConfig {
        vendor_dir: vendor_dir.clone(),
        base_dir: working_dir.clone(),
        optimize: args.optimize || args.classmap_authoritative,
        apcu: args.apcu,
        authoritative: args.classmap_authoritative,
        suffix,
    };

    let generator = AutoloadGenerator::new(config);
    generator.generate(&packages, root_autoload.as_ref())
        .context("Failed to generate autoloader")?;

    if args.optimize || args.classmap_authoritative {
        println!("{} Generated optimized autoload files",
            style("Success:").green().bold()
        );
    } else {
        println!("{} Generated autoload files",
            style("Success:").green().bold()
        );
    }

    Ok(0)
}

/// Convert a LockedPackage to a PackageAutoload
fn locked_package_to_autoload(lp: &phpx_pm::json::LockedPackage) -> PackageAutoload {
    let autoload = convert_lock_autoload(&lp.autoload);

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

fn convert_lock_autoload(lock_autoload: &phpx_pm::json::LockAutoload) -> Autoload {
    let mut autoload = Autoload::default();

    for (namespace, value) in &lock_autoload.psr4 {
        let paths = json_value_to_paths(value);
        autoload.psr4.insert(namespace.clone(), paths);
    }

    for (namespace, value) in &lock_autoload.psr0 {
        let paths = json_value_to_paths(value);
        autoload.psr0.insert(namespace.clone(), paths);
    }

    autoload.classmap = lock_autoload.classmap.clone();
    autoload.files = lock_autoload.files.clone();
    autoload.exclude_from_classmap = lock_autoload.exclude_from_classmap.clone();

    autoload
}

/// Convert JSON value to AutoloadPath
fn json_value_to_paths(value: &serde_json::Value) -> phpx_pm::package::AutoloadPath {
    use phpx_pm::package::AutoloadPath;

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

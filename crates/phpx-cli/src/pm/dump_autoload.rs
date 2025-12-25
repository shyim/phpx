//! Dump-autoload command - regenerate the autoloader.

use anyhow::{Context, Result};
use clap::Args;
use console::style;
use std::collections::HashMap;
use std::path::PathBuf;

use phpx_pm::{
    autoload::{AutoloadConfig, AutoloadGenerator, PackageAutoload, RootPackageInfo},
    json::{ComposerJson, ComposerLock, LockedPackage},
    package::Autoload,
    plugin::PluginRegistry,
    repository::get_head_commit,
    Package,
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

    // Load composer.json
    let composer_json_path = working_dir.join("composer.json");
    let composer_json: Option<ComposerJson> = if composer_json_path.exists() {
        let content = std::fs::read_to_string(&composer_json_path)?;
        serde_json::from_str(&content).ok()
    } else {
        None
    };

    // Load composer.lock to get packages and content-hash for suffix
    let lock_path = working_dir.join("composer.lock");
    let (packages, suffix, installed_packages, aliases_map) = if lock_path.exists() {
        let content = std::fs::read_to_string(&lock_path)
            .context("Failed to read composer.lock")?;
        let lock: ComposerLock = serde_json::from_str(&content)
            .context("Failed to parse composer.lock")?;

        // Build alias map from lock file
        let mut aliases_map: HashMap<String, Vec<String>> = HashMap::new();
        for alias in &lock.aliases {
            aliases_map.entry(alias.package.clone())
                .or_default()
                .push(alias.alias.clone());
        }

        let dev_mode = !args.no_dev;

        let mut pkgs: Vec<PackageAutoload> = lock.packages.iter()
            .map(|lp| locked_package_to_autoload(lp, false, &aliases_map))
            .collect();

        // Build list of installed packages for plugin registry
        let mut installed: Vec<Package> = lock.packages.iter()
            .map(|lp| Package::new(&lp.name, &lp.version))
            .collect();

        if dev_mode {
            pkgs.extend(lock.packages_dev.iter().map(|lp| locked_package_to_autoload(lp, true, &aliases_map)));
            installed.extend(lock.packages_dev.iter().map(|lp| Package::new(&lp.name, &lp.version)));
        }

        // Use the content-hash as the suffix
        let suffix = if !lock.content_hash.is_empty() {
            Some(lock.content_hash.clone())
        } else {
            None
        };

        (pkgs, suffix, installed, aliases_map)
    } else {
        (Vec::new(), None, Vec::new(), HashMap::new())
    };

    // Get root autoload from composer.json
    let root_autoload: Option<Autoload> = composer_json.as_ref()
        .map(|cj| cj.autoload.clone().into())
        .filter(|al: &Autoload| !al.is_empty());

    // Build root package info
    let root_package = composer_json.as_ref().map(|cj| {
        let name = cj.name.clone().unwrap_or_else(|| "__root__".to_string());
        let root_aliases = aliases_map.get(&name).cloned().unwrap_or_default();
        let reference = get_head_commit(&working_dir);
        RootPackageInfo {
            name,
            pretty_version: cj.version.clone().unwrap_or_else(|| "dev-main".to_string()),
            version: cj.version.clone().unwrap_or_else(|| "dev-main".to_string()),
            reference,
            package_type: cj.package_type.clone(),
            aliases: root_aliases,
            dev_mode: !args.no_dev,
        }
    });

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
    generator.generate(&packages, root_autoload.as_ref(), root_package.as_ref())
        .context("Failed to generate autoloader")?;

    // Run plugin hooks (post-autoload-dump)
    if let Some(ref cj) = composer_json {
        let plugin_registry = PluginRegistry::new();
        plugin_registry.run_post_autoload_dump(
            &vendor_dir,
            &working_dir,
            cj,
            &installed_packages,
        ).context("Failed to run plugin hooks")?;
    }

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
fn locked_package_to_autoload(lp: &LockedPackage, is_dev: bool, aliases_map: &HashMap<String, Vec<String>>) -> PackageAutoload {
    let autoload = convert_lock_autoload(&lp.autoload);

    let requires: Vec<String> = lp.require.keys()
        .filter(|k| *k != "php" && !k.starts_with("ext-") && !k.starts_with("lib-"))
        .cloned()
        .collect();

    // Get the reference from source or dist
    let reference = lp.source.as_ref()
        .map(|s| s.reference.clone())
        .or_else(|| lp.dist.as_ref().and_then(|d| d.reference.clone()));

    // Get aliases for this package
    let aliases = aliases_map.get(&lp.name).cloned().unwrap_or_default();

    PackageAutoload {
        name: lp.name.clone(),
        autoload,
        install_path: lp.name.clone(),
        requires,
        pretty_version: Some(lp.version.clone()),
        version: Some(lp.version.clone()), // Use same as pretty_version for now
        reference,
        package_type: lp.package_type.clone(),
        dev_requirement: is_dev,
        aliases,
        replaces: lp.replace.clone(),
        provides: lp.provide.clone(),
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

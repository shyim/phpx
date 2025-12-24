//! Add command - add and install a package.

use anyhow::{Context, Result};
use clap::Args;
use console::style;
use std::path::PathBuf;

use phpx_pm::json::ComposerJson;

#[derive(Args, Debug)]
pub struct AddArgs {
    /// Packages to require (e.g., vendor/package:^1.0)
    #[arg(value_name = "PACKAGES", required = true)]
    pub packages: Vec<String>,

    /// Add as development dependency
    #[arg(long)]
    pub dev: bool,

    /// Prefer source installation
    #[arg(long)]
    pub prefer_source: bool,

    /// Prefer dist installation
    #[arg(long)]
    pub prefer_dist: bool,

    /// Run in dry-run mode
    #[arg(long)]
    pub dry_run: bool,

    /// Skip autoloader generation
    #[arg(long)]
    pub no_autoloader: bool,

    /// Skip script execution
    #[arg(long)]
    pub no_scripts: bool,

    /// Do not run update after adding
    #[arg(long)]
    pub no_update: bool,

    /// Optimize autoloader
    #[arg(short = 'o', long)]
    pub optimize_autoloader: bool,

    /// Working directory
    #[arg(short = 'd', long, default_value = ".")]
    pub working_dir: PathBuf,
}

pub async fn execute(args: AddArgs) -> Result<i32> {
    let working_dir = args.working_dir.canonicalize()
        .context("Failed to resolve working directory")?;

    let json_path = working_dir.join("composer.json");

    // Load or create composer.json
    let mut composer_json: ComposerJson = if json_path.exists() {
        let content = std::fs::read_to_string(&json_path)
            .context("Failed to read composer.json")?;
        serde_json::from_str(&content)
            .context("Failed to parse composer.json")?
    } else {
        eprintln!("{} No composer.json found. Creating one.",
            style("Info:").cyan()
        );
        ComposerJson::default()
    };

    println!("{} Adding packages", style("Composer").green().bold());

    if args.dry_run {
        println!("{} Running in dry-run mode", style("Info:").cyan());
    }

    // Parse package specifications
    for spec in &args.packages {
        let (name, constraint) = parse_package_spec(spec);

        println!("  {} {} {}",
            style("+").green(),
            style(&name).white().bold(),
            style(&constraint).yellow()
        );

        if args.dev {
            composer_json.require_dev.insert(name, constraint);
        } else {
            composer_json.require.insert(name, constraint);
        }
    }

    // Write updated composer.json
    if !args.dry_run {
        let content = serde_json::to_string_pretty(&composer_json)
            .context("Failed to serialize composer.json")?;
        std::fs::write(&json_path, content)
            .context("Failed to write composer.json")?;
    }

    // Run update if not disabled
    if !args.no_update {
        println!("{} Running update...", style("Info:").cyan());

        let update_args = crate::update::UpdateArgs {
            packages: args.packages.iter()
                .map(|s| parse_package_spec(s).0)
                .collect(),
            prefer_source: args.prefer_source,
            prefer_dist: args.prefer_dist,
            dry_run: args.dry_run,
            no_dev: false,
            no_autoloader: args.no_autoloader,
            no_scripts: args.no_scripts,
            no_progress: false,
            with_dependencies: true,
            with_all_dependencies: false,
            prefer_stable: true,
            prefer_lowest: false,
            lock: false,
            optimize_autoloader: args.optimize_autoloader,
            working_dir: working_dir.clone(),
        };

        return crate::update::execute(update_args).await;
    }

    println!("{} Packages added to composer.json",
        style("Success:").green().bold()
    );

    Ok(0)
}

/// Parse a package specification (vendor/package:^1.0 or vendor/package)
fn parse_package_spec(spec: &str) -> (String, String) {
    if let Some(pos) = spec.find(':') {
        let name = spec[..pos].to_string();
        let constraint = spec[pos + 1..].to_string();
        (name, constraint)
    } else {
        // Default to any version
        (spec.to_string(), "*".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_package_spec() {
        let (name, constraint) = parse_package_spec("vendor/package:^1.0");
        assert_eq!(name, "vendor/package");
        assert_eq!(constraint, "^1.0");

        let (name, constraint) = parse_package_spec("vendor/package");
        assert_eq!(name, "vendor/package");
        assert_eq!(constraint, "*");
    }
}

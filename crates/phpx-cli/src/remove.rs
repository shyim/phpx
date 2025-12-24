//! Remove command - remove a package from the project.

use anyhow::{Context, Result};
use clap::Args;
use console::style;
use std::path::PathBuf;

use phpx_pm::json::ComposerJson;

#[derive(Args, Debug)]
pub struct RemoveArgs {
    /// Packages to remove
    #[arg(value_name = "PACKAGES", required = true)]
    pub packages: Vec<String>,

    /// Remove from development dependencies
    #[arg(long)]
    pub dev: bool,

    /// Run in dry-run mode
    #[arg(long)]
    pub dry_run: bool,

    /// Do not run update after removing
    #[arg(long)]
    pub no_update: bool,

    /// Skip autoloader generation
    #[arg(long)]
    pub no_autoloader: bool,

    /// Skip script execution
    #[arg(long)]
    pub no_scripts: bool,

    /// Optimize autoloader
    #[arg(short = 'o', long)]
    pub optimize_autoloader: bool,

    /// Working directory
    #[arg(short = 'd', long, default_value = ".")]
    pub working_dir: PathBuf,
}

pub async fn execute(args: RemoveArgs) -> Result<i32> {
    let working_dir = args.working_dir.canonicalize()
        .context("Failed to resolve working directory")?;

    let json_path = working_dir.join("composer.json");

    if !json_path.exists() {
        eprintln!("{} No composer.json found in {}",
            style("Error:").red().bold(),
            working_dir.display()
        );
        return Ok(1);
    }

    // Load composer.json
    let content = std::fs::read_to_string(&json_path)
        .context("Failed to read composer.json")?;
    let mut composer_json: ComposerJson = serde_json::from_str(&content)
        .context("Failed to parse composer.json")?;

    println!("{} Removing packages", style("Composer").green().bold());

    if args.dry_run {
        println!("{} Running in dry-run mode", style("Info:").cyan());
    }

    let mut removed = Vec::new();

    for name in &args.packages {
        // Try to remove from require or require-dev
        let was_in_require = composer_json.require.remove(name).is_some();
        let was_in_dev = composer_json.require_dev.remove(name).is_some();

        if was_in_require || was_in_dev {
            println!("  {} {}",
                style("-").red(),
                style(name).white().bold()
            );
            removed.push(name.clone());
        } else {
            println!("  {} {} is not installed",
                style("!").yellow(),
                style(name).white()
            );
        }
    }

    if removed.is_empty() {
        println!("{} Nothing to remove", style("Info:").cyan());
        return Ok(0);
    }

    // Write updated composer.json
    if !args.dry_run {
        let content = serde_json::to_string_pretty(&composer_json)
            .context("Failed to serialize composer.json")?;
        std::fs::write(&json_path, content)
            .context("Failed to write composer.json")?;
    }

    // Run update to regenerate lock file and remove packages
    if !args.no_update {
        println!("{} Running update...", style("Info:").cyan());

        let update_args = super::update::UpdateArgs {
            packages: vec![],
            prefer_source: false,
            prefer_dist: true,
            dry_run: args.dry_run,
            no_dev: false,
            no_autoloader: args.no_autoloader,
            no_scripts: args.no_scripts,
            no_progress: false,
            with_dependencies: false,
            with_all_dependencies: false,
            prefer_stable: true,
            prefer_lowest: false,
            lock: false,
            optimize_autoloader: args.optimize_autoloader,
            working_dir: working_dir.clone(),
        };

        return super::update::execute(update_args).await;
    }

    println!("{} {} packages removed from composer.json",
        style("Success:").green().bold(),
        removed.len()
    );

    Ok(0)
}

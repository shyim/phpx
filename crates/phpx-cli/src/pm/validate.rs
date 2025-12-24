//! Validate command - validate composer.json and composer.lock.

use anyhow::{Context, Result};
use clap::Args;
use console::style;
use std::path::PathBuf;

use phpx_pm::json::{ComposerJson, ComposerLock};

#[derive(Args, Debug)]
pub struct ValidateArgs {
    /// Only validate composer.json, don't check lock file
    #[arg(long)]
    pub no_check_lock: bool,

    /// Validate all packages, not just root
    #[arg(long)]
    pub no_check_all: bool,

    /// Strict mode: treat warnings as errors
    #[arg(long)]
    pub strict: bool,

    /// Output as JSON
    #[arg(long)]
    pub format_json: bool,

    /// Check only requirements that are in composer.json
    #[arg(long)]
    pub no_check_version: bool,

    /// Disables all publishing checks
    #[arg(long)]
    pub no_check_publish: bool,

    /// Working directory
    #[arg(short = 'd', long, default_value = ".")]
    pub working_dir: PathBuf,
}

pub async fn execute(args: ValidateArgs) -> Result<i32> {
    let working_dir = args.working_dir.canonicalize()
        .context("Failed to resolve working directory")?;

    let json_path = working_dir.join("composer.json");
    let lock_path = working_dir.join("composer.lock");

    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    // Check composer.json exists
    if !json_path.exists() {
        errors.push(format!("composer.json not found in {}", working_dir.display()));
        return print_results(&errors, &warnings, args.format_json, args.strict);
    }

    // Validate composer.json
    let json_content = std::fs::read_to_string(&json_path)
        .context("Failed to read composer.json")?;

    let composer_json: ComposerJson = match serde_json::from_str(&json_content) {
        Ok(json) => json,
        Err(e) => {
            errors.push(format!("composer.json is not valid JSON: {}", e));
            return print_results(&errors, &warnings, args.format_json, args.strict);
        }
    };

    // Validate required fields
    if composer_json.name.is_none() && !args.no_check_publish {
        warnings.push("No 'name' property defined".to_string());
    }

    if composer_json.description.is_none() && !args.no_check_publish {
        warnings.push("No 'description' property defined".to_string());
    }

    // Check for invalid package names in require
    for name in composer_json.require.keys() {
        if !is_valid_package_name(name) && !is_platform_package(name) {
            warnings.push(format!("Invalid package name '{}' in require", name));
        }
    }

    for name in composer_json.require_dev.keys() {
        if !is_valid_package_name(name) && !is_platform_package(name) {
            warnings.push(format!("Invalid package name '{}' in require-dev", name));
        }
    }

    // Validate composer.lock if it exists
    if !args.no_check_lock && lock_path.exists() {
        let lock_content = std::fs::read_to_string(&lock_path)
            .context("Failed to read composer.lock")?;

        match serde_json::from_str::<ComposerLock>(&lock_content) {
            Ok(_lock) => {
                // Basic lock file validation passed
            }
            Err(e) => {
                errors.push(format!("composer.lock is not valid: {}", e));
            }
        }
    } else if !args.no_check_lock && !lock_path.exists() && !composer_json.require.is_empty() {
        warnings.push("composer.lock is not present. Run 'phpx composer install' to generate it.".to_string());
    }

    print_results(&errors, &warnings, args.format_json, args.strict)
}

fn print_results(
    errors: &[String],
    warnings: &[String],
    as_json: bool,
    strict: bool,
) -> Result<i32> {
    if as_json {
        let result = serde_json::json!({
            "valid": errors.is_empty() && (!strict || warnings.is_empty()),
            "errors": errors,
            "warnings": warnings
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        for error in errors {
            eprintln!("{} {}", style("Error:").red().bold(), error);
        }

        for warning in warnings {
            println!("{} {}", style("Warning:").yellow().bold(), warning);
        }

        if errors.is_empty() && warnings.is_empty() {
            println!("{} composer.json is valid", style("Success:").green().bold());
        } else if errors.is_empty() {
            println!("{} composer.json is valid with {} warning(s)",
                style("Success:").green().bold(),
                warnings.len()
            );
        }
    }

    if !errors.is_empty() {
        return Ok(2);
    }

    if strict && !warnings.is_empty() {
        return Ok(1);
    }

    Ok(0)
}

fn is_valid_package_name(name: &str) -> bool {
    // vendor/package format
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() != 2 {
        return false;
    }

    let vendor = parts[0];
    let package = parts[1];

    // Basic validation
    !vendor.is_empty() && !package.is_empty()
        && vendor.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        && package.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
}

fn is_platform_package(name: &str) -> bool {
    name == "php"
        || name.starts_with("ext-")
        || name.starts_with("lib-")
        || name == "composer"
        || name.starts_with("composer-")
}

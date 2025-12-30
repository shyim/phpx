use anyhow::{Context, Result};
use clap::Args;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use pox_pm::{
    Repository,
    config::Config,
    json::{ComposerJson, ComposerLock},
    package::detect_root_version,
    repository::RepositoryUtils,
};

#[derive(Args, Debug)]
pub struct LicensesArgs {
    /// Output format: text, json or summary
    #[arg(short = 'f', long, default_value = "text")]
    pub format: String,

    /// Disables search in require-dev packages
    #[arg(long)]
    pub no_dev: bool,

    /// Shows licenses from the lock file instead of installed packages
    #[arg(long)]
    pub locked: bool,

    /// Working directory
    #[arg(short = 'd', long, default_value = ".")]
    pub working_dir: PathBuf,
}

pub async fn execute(args: LicensesArgs) -> Result<i32> {
    let working_dir = args
        .working_dir
        .canonicalize()
        .context("Failed to resolve working directory")?;

    if args.format != "text" && args.format != "json" && args.format != "summary" {
        eprintln!(
            "Error: Unsupported format '{}'. See help for supported formats.",
            args.format
        );
        return Ok(1);
    }

    let json_path = working_dir.join("composer.json");
    let composer_json: ComposerJson = if json_path.exists() {
        let content = std::fs::read_to_string(&json_path)?;
        serde_json::from_str(&content)?
    } else {
        eprintln!("Error: composer.json not found in working directory");
        return Ok(1);
    };

    let config = Config::build(Some(&working_dir), true)?;
    let vendor_dir = working_dir.join(&config.vendor_dir);

    let packages: Vec<Arc<pox_pm::Package>> = if args.locked {
        let lock_path = working_dir.join("composer.lock");
        if !lock_path.exists() {
            eprintln!("Error: Valid composer.json and composer.lock files are required to run this command with --locked");
            return Ok(1);
        }

        let lock_content = std::fs::read_to_string(&lock_path)?;
        let lock: ComposerLock = serde_json::from_str(&lock_content)?;

        let mut locked_packages = lock.packages;
        if !args.no_dev {
            locked_packages.extend(lock.packages_dev);
        }
        locked_packages
            .into_iter()
            .map(|lp| Arc::new(pox_pm::Package::from(lp)))
            .collect()
    } else {
        let installed_repo =
            Arc::new(pox_pm::repository::InstalledRepository::new(vendor_dir.clone()));
        installed_repo.load().await.ok();
        let all_packages = installed_repo.get_packages().await;

        if args.no_dev {
            RepositoryUtils::filter_required_packages(&all_packages, &composer_json)
        } else {
            all_packages
        }
    };

    let mut packages: Vec<_> = packages.into_iter().collect();
    packages.sort_by(|a, b| a.name.cmp(&b.name));

    let root_name = composer_json.name.as_deref().unwrap_or("__root__");
    let branch_aliases = composer_json.get_branch_aliases();
    let root_version_info = detect_root_version(
        &working_dir,
        composer_json.version.as_deref(),
        &branch_aliases,
    );

    let root_version = if root_version_info.pretty_version.contains("dev") {
        if let Some(git_ref) = get_short_git_ref(&working_dir) {
            format!("{} {}", root_version_info.pretty_version, git_ref)
        } else {
            root_version_info.pretty_version
        }
    } else {
        root_version_info.pretty_version
    };
    let root_licenses = composer_json.licenses();

    match args.format.as_str() {
        "text" => {
            println!("Name: {}", root_name);
            println!("Version: {}", root_version);
            println!(
                "Licenses: {}",
                if root_licenses.is_empty() {
                    "none".to_string()
                } else {
                    root_licenses.join(", ")
                }
            );
            println!("Dependencies:");
            println!();

            let name_width = packages
                .iter()
                .map(|p| p.name.len())
                .max()
                .unwrap_or(4)
                .max(4);
            let version_width = packages
                .iter()
                .map(|p| p.pretty_version.as_deref().unwrap_or(&p.version).len())
                .max()
                .unwrap_or(7)
                .max(7);

            println!(
                "{:<name_width$} {:<version_width$} {}",
                "Name",
                "Version",
                "Licenses",
                name_width = name_width,
                version_width = version_width
            );

            for package in &packages {
                let version = package.pretty_version.as_deref().unwrap_or(&package.version);
                let licenses = if package.license.is_empty() {
                    "none".to_string()
                } else {
                    package.license.join(", ")
                };

                println!(
                    "{:<name_width$} {:<version_width$} {}",
                    package.name,
                    version,
                    licenses,
                    name_width = name_width,
                    version_width = version_width
                );
            }
        }
        "json" => {
            let mut dependencies: serde_json::Map<String, serde_json::Value> =
                serde_json::Map::new();

            for package in &packages {
                let version = package.pretty_version.as_deref().unwrap_or(&package.version);
                dependencies.insert(
                    package.name.clone(),
                    serde_json::json!({
                        "version": version,
                        "license": package.license,
                    }),
                );
            }

            let output = serde_json::json!({
                "name": root_name,
                "version": root_version,
                "license": root_licenses,
                "dependencies": dependencies,
            });

            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        "summary" => {
            let mut used_licenses: HashMap<String, usize> = HashMap::new();

            for package in &packages {
                let licenses = if package.license.is_empty() {
                    vec!["none".to_string()]
                } else {
                    package.license.clone()
                };

                for license in licenses {
                    *used_licenses.entry(license).or_insert(0) += 1;
                }
            }

            let mut license_counts: Vec<_> = used_licenses.into_iter().collect();
            license_counts.sort_by(|a, b| b.1.cmp(&a.1));

            println!(" ----------------------- ----------------------- ");
            println!("  License                 Number of dependencies ");
            println!(" ----------------------- ----------------------- ");

            for (license, count) in license_counts {
                println!("  {:<22}  {:<22}", license, count);
            }

            println!(" ----------------------- ----------------------- ");
        }
        _ => unreachable!(),
    }

    Ok(0)
}

fn get_short_git_ref(path: &std::path::Path) -> Option<String> {
    let git_dir = path.join(".git");
    if !git_dir.exists() {
        return None;
    }

    let head_path = git_dir.join("HEAD");
    if !head_path.exists() {
        return None;
    }

    let head_content = std::fs::read_to_string(&head_path).ok()?;
    let head = head_content.trim();

    if let Some(ref_path) = head.strip_prefix("ref: ") {
        let ref_file = git_dir.join(ref_path);
        if ref_file.exists() {
            let commit = std::fs::read_to_string(&ref_file).ok()?;
            return Some(commit.trim().chars().take(7).collect());
        }
        let packed_refs = git_dir.join("packed-refs");
        if packed_refs.exists() {
            let content = std::fs::read_to_string(&packed_refs).ok()?;
            for line in content.lines() {
                if line.ends_with(ref_path) {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if let Some(commit) = parts.first() {
                        return Some(commit.chars().take(7).collect());
                    }
                }
            }
        }
        return None;
    }

    Some(head.chars().take(7).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_get_short_git_ref_no_git_dir() {
        let temp_dir = TempDir::new().unwrap();
        let result = get_short_git_ref(temp_dir.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_get_short_git_ref_detached_head() {
        let temp_dir = TempDir::new().unwrap();
        let git_dir = temp_dir.path().join(".git");
        fs::create_dir(&git_dir).unwrap();
        fs::write(git_dir.join("HEAD"), "abc1234567890def1234567890abcdef12345678").unwrap();

        let result = get_short_git_ref(temp_dir.path());
        assert_eq!(result, Some("abc1234".to_string()));
    }

    #[test]
    fn test_get_short_git_ref_symbolic_ref() {
        let temp_dir = TempDir::new().unwrap();
        let git_dir = temp_dir.path().join(".git");
        fs::create_dir_all(git_dir.join("refs/heads")).unwrap();
        fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(git_dir.join("refs/heads/main"), "fedcba9876543210fedcba9876543210fedcba98").unwrap();

        let result = get_short_git_ref(temp_dir.path());
        assert_eq!(result, Some("fedcba9".to_string()));
    }

    #[test]
    fn test_get_short_git_ref_packed_refs() {
        let temp_dir = TempDir::new().unwrap();
        let git_dir = temp_dir.path().join(".git");
        fs::create_dir(&git_dir).unwrap();
        fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(git_dir.join("packed-refs"), "# pack-refs with: peeled fully-peeled sorted\n\
                          1234567890abcdef1234567890abcdef12345678 refs/heads/main\n").unwrap();

        let result = get_short_git_ref(temp_dir.path());
        assert_eq!(result, Some("1234567".to_string()));
    }

    #[test]
    fn test_get_short_git_ref_missing_ref() {
        let temp_dir = TempDir::new().unwrap();
        let git_dir = temp_dir.path().join(".git");
        fs::create_dir(&git_dir).unwrap();
        fs::write(git_dir.join("HEAD"), "ref: refs/heads/nonexistent\n").unwrap();

        let result = get_short_git_ref(temp_dir.path());
        assert!(result.is_none());
    }
}

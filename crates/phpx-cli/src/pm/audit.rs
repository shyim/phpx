use anyhow::{Context, Result};
use clap::Args;
use colored::Colorize;
use phpx_pm::json::{ComposerLock, LockedPackage};
use phpx_pm::cache::Cache;
use phpx_pm::config::Config;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Args, Debug)]
pub struct AuditArgs {
    /// Disables auditing of require-dev packages
    #[arg(long)]
    pub no_dev: bool,

    /// Output format (table, plain, json, or summary)
    #[arg(short, long, default_value = "table")]
    pub format: String,

    /// Audit based on the lock file instead of the installed packages
    #[arg(long)]
    pub locked: bool,

    /// Behavior on abandoned packages (ignore, report, or fail)
    #[arg(long, value_parser = ["ignore", "report", "fail"])]
    pub abandoned: Option<String>,

    /// Working directory
    #[arg(short = 'd', long, default_value = ".")]
    pub working_dir: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct SecurityAdvisoriesResponse {
    advisories: HashMap<String, Vec<SecurityAdvisory>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedPackageAdvisories {
    advisories: Vec<SecurityAdvisory>,
    cached_at: u64, // Unix timestamp
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SecurityAdvisory {
    #[serde(rename = "advisoryId")]
    advisory_id: String,
    #[serde(rename = "packageName")]
    package_name: String,
    title: String,
    #[serde(default)]
    cve: Option<String>,
    #[serde(default)]
    link: Option<String>,
    #[serde(default)]
    severity: Option<String>,
    #[serde(rename = "affectedVersions")]
    affected_versions: String,
    #[serde(rename = "reportedAt")]
    reported_at: String,
    #[serde(default)]
    sources: Vec<AdvisorySource>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct AdvisorySource {
    #[serde(rename = "name")]
    _name: String,
    #[serde(rename = "remoteId")]
    _remote_id: String,
}

pub async fn execute(args: AuditArgs) -> Result<i32> {
    let working_dir = args
        .working_dir
        .canonicalize()
        .context("Failed to resolve working directory")?;

    // Load composer.lock
    let lock_path = working_dir.join("composer.lock");
    let lock: ComposerLock = if lock_path.exists() {
        let content = std::fs::read_to_string(&lock_path)?;
        serde_json::from_str(&content)
            .context("Failed to parse composer.lock")?
    } else {
        return Err(anyhow::anyhow!("No composer.lock found. Run 'install' or 'update' first."));
    };

    // Get packages to audit
    let packages: Vec<String> = if args.no_dev {
        lock.packages.iter().map(|p| p.name.clone()).collect()
    } else {
        lock.packages
            .iter()
            .chain(lock.packages_dev.iter())
            .map(|p| p.name.clone())
            .collect()
    };

    if packages.is_empty() {
        println!("{}", "No packages - skipping audit.".yellow());
        return Ok(0);
    }

    // Load config to get cache directory
    let config = Config::build(Some(&working_dir), true)?;
    let cache_dir = config.cache_dir
        .context("Cache directory not configured")?
        .join("audit");
    let cache = Cache::new(cache_dir);

    // Cache TTL: 10 minutes
    let cache_ttl = Duration::from_secs(10 * 60);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Check which packages have valid cache and which need fresh data
    let mut cached_advisories: HashMap<String, Vec<SecurityAdvisory>> = HashMap::new();
    let mut packages_to_fetch: Vec<String> = Vec::new();

    for package in &packages {
        let cache_key = format!("advisory/{}", package.replace('/', "-"));

        // Check if cache exists and is fresh
        if let Ok(Some(age)) = cache.age(&cache_key) {
            if age < cache_ttl {
                // Cache is fresh, try to load it
                if let Ok(Some(data)) = cache.read(&cache_key) {
                    if let Ok(cached) = serde_json::from_slice::<CachedPackageAdvisories>(&data) {
                        if !cached.advisories.is_empty() {
                            cached_advisories.insert(package.clone(), cached.advisories);
                            continue;
                        }
                    }
                }
            }
        }

        // No valid cache, need to fetch
        packages_to_fetch.push(package.clone());
    }

    // Fetch fresh data for packages not in cache
    let mut fresh_advisories: HashMap<String, Vec<SecurityAdvisory>> = HashMap::new();

    if !packages_to_fetch.is_empty() {
        let api_url = "https://packagist.org/api/security-advisories/";

        let form_data = packages_to_fetch
            .iter()
            .map(|p| format!("packages[]={}", p))
            .collect::<Vec<_>>()
            .join("&");

        let client = reqwest::Client::new();
        let response = client
            .post(api_url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(form_data)
            .send()
            .await
            .context("Failed to query security advisories API")?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Security advisories API returned status: {}",
                response.status()
            ));
        }

        let api_response: SecurityAdvisoriesResponse = response
            .json()
            .await
            .context("Failed to parse security advisories response")?;

        fresh_advisories = api_response.advisories;

        // Cache the fresh data
        for package in &packages_to_fetch {
            let cache_key = format!("advisory/{}", package.replace('/', "-"));
            let advisories = fresh_advisories.get(package).cloned().unwrap_or_default();

            let cached = CachedPackageAdvisories {
                advisories,
                cached_at: now,
            };

            if let Ok(data) = serde_json::to_vec(&cached) {
                let _ = cache.write(&cache_key, &data);
            }
        }
    }

    // Merge cached and fresh advisories
    let mut all_advisories = cached_advisories;
    for (package, advisories) in fresh_advisories {
        all_advisories.insert(package, advisories);
    }

    // Filter to only include packages we're checking
    let advisories_response = SecurityAdvisoriesResponse {
        advisories: all_advisories
            .into_iter()
            .filter(|(k, v)| packages.contains(k) && !v.is_empty())
            .collect(),
    };

    // Check for abandoned packages
    let abandoned_packages: Vec<_> = if let Some(abandoned_behavior) = &args.abandoned {
        if abandoned_behavior != "ignore" {
            lock.packages
                .iter()
                .chain(if args.no_dev {
                    [].iter()
                } else {
                    lock.packages_dev.iter()
                })
                .filter(|p| p.is_abandoned())
                .collect()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    // Display results
    let has_vulnerabilities = !advisories_response.advisories.is_empty();
    let has_abandoned = !abandoned_packages.is_empty();

    match args.format.as_str() {
        "json" => {
            output_json(&advisories_response, &abandoned_packages)?;
        }
        "plain" => {
            output_plain(&advisories_response, &abandoned_packages)?;
        }
        "summary" => {
            output_summary(&advisories_response)?;
        }
        _ => {
            // table format (default)
            output_table(&advisories_response, &abandoned_packages)?;
        }
    }

    // Return exit code
    let mut exit_code = 0;
    if has_vulnerabilities {
        exit_code |= 1; // STATUS_VULNERABLE
    }
    if has_abandoned && args.abandoned == Some("fail".to_string()) {
        exit_code |= 2; // STATUS_ABANDONED
    }

    Ok(exit_code)
}

fn output_json(
    response: &SecurityAdvisoriesResponse,
    abandoned_packages: &[&LockedPackage],
) -> Result<()> {
    #[derive(Serialize)]
    struct JsonOutput {
        advisories: HashMap<String, Vec<SecurityAdvisory>>,
        abandoned: HashMap<String, Option<String>>,
    }

    let abandoned_map: HashMap<String, Option<String>> = abandoned_packages
        .iter()
        .map(|p| (p.name.clone(), p.abandoned_replacement().map(String::from)))
        .collect();

    let output = JsonOutput {
        advisories: response.advisories.clone(),
        abandoned: abandoned_map,
    };

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn output_table(
    response: &SecurityAdvisoriesResponse,
    abandoned_packages: &[&LockedPackage],
) -> Result<()> {
    let total_advisories: usize = response.advisories.values().map(|v| v.len()).sum();
    let affected_packages = response.advisories.len();

    if total_advisories > 0 {
        let plurality = if total_advisories == 1 { "y" } else { "ies" };
        let pkg_plurality = if affected_packages == 1 { "" } else { "s" };

        println!(
            "{}",
            format!(
                "Found {} security vulnerability advisor{} affecting {} package{}:",
                total_advisories, plurality, affected_packages, pkg_plurality
            )
            .red()
            .bold()
        );
        println!();

        for advisories in response.advisories.values() {
            for advisory in advisories {
                println!("{}", "─".repeat(80).bright_black());
                println!("{}: {}", "Package".bold(), advisory.package_name);
                println!(
                    "{}: {}",
                    "Severity".bold(),
                    colorize_severity(advisory.severity.as_deref())
                );
                println!("{}: {}", "Advisory ID".bold(), advisory.advisory_id);
                println!(
                    "{}: {}",
                    "CVE".bold(),
                    advisory.cve.as_deref().unwrap_or("NO CVE")
                );
                println!("{}: {}", "Title".bold(), advisory.title);
                if let Some(link) = &advisory.link {
                    println!("{}: {}", "URL".bold(), link);
                }
                println!(
                    "{}: {}",
                    "Affected versions".bold(),
                    advisory.affected_versions
                );
                println!("{}: {}", "Reported at".bold(), advisory.reported_at);
                println!();
            }
        }
    } else {
        println!(
            "{}",
            "No security vulnerability advisories found.".green().bold()
        );
    }

    if !abandoned_packages.is_empty() {
        println!(
            "{}",
            format!("Found {} abandoned package{}:", abandoned_packages.len(), if abandoned_packages.len() > 1 { "s" } else { "" })
                .yellow()
                .bold()
        );
        println!();

        for pkg in abandoned_packages {
            let replacement = pkg
                .abandoned_replacement()
                .map(|r| format!("Use {} instead", r))
                .unwrap_or_else(|| "No replacement was suggested".to_string());
            println!("  {} is abandoned. {}", pkg.name.yellow(), replacement);
        }
    }

    Ok(())
}

fn output_plain(
    response: &SecurityAdvisoriesResponse,
    abandoned_packages: &[&LockedPackage],
) -> Result<()> {
    let total_advisories: usize = response.advisories.values().map(|v| v.len()).sum();
    let affected_packages = response.advisories.len();

    if total_advisories > 0 {
        let plurality = if total_advisories == 1 { "y" } else { "ies" };
        let pkg_plurality = if affected_packages == 1 { "" } else { "s" };

        eprintln!(
            "Found {} security vulnerability advisor{} affecting {} package{}:",
            total_advisories, plurality, affected_packages, pkg_plurality
        );

        let mut first = true;
        for advisories in response.advisories.values() {
            for advisory in advisories {
                if !first {
                    eprintln!("--------");
                }
                eprintln!("Package: {}", advisory.package_name);
                eprintln!(
                    "Severity: {}",
                    advisory.severity.as_deref().unwrap_or("")
                );
                eprintln!("Advisory ID: {}", advisory.advisory_id);
                eprintln!("CVE: {}", advisory.cve.as_deref().unwrap_or("NO CVE"));
                eprintln!("Title: {}", advisory.title);
                eprintln!("URL: {}", advisory.link.as_deref().unwrap_or(""));
                eprintln!("Affected versions: {}", advisory.affected_versions);
                eprintln!("Reported at: {}", advisory.reported_at);
                first = false;
            }
        }
    } else {
        eprintln!("No security vulnerability advisories found.");
    }

    if !abandoned_packages.is_empty() {
        eprintln!(
            "Found {} abandoned package{}:",
            abandoned_packages.len(),
            if abandoned_packages.len() > 1 { "s" } else { "" }
        );

        for pkg in abandoned_packages {
            let replacement = pkg
                .abandoned_replacement()
                .map(|r| format!("Use {} instead", r))
                .unwrap_or_else(|| "No replacement was suggested".to_string());
            eprintln!("{} is abandoned. {}", pkg.name, replacement);
        }
    }

    Ok(())
}

fn output_summary(response: &SecurityAdvisoriesResponse) -> Result<()> {
    let total_advisories: usize = response.advisories.values().map(|v| v.len()).sum();
    let affected_packages = response.advisories.len();

    if total_advisories > 0 {
        let plurality = if total_advisories == 1 { "y" } else { "ies" };
        let pkg_plurality = if affected_packages == 1 { "" } else { "s" };

        eprintln!(
            "Found {} security vulnerability advisor{} affecting {} package{}.",
            total_advisories, plurality, affected_packages, pkg_plurality
        );
        eprintln!("Run \"phpx pm audit\" for a full list of advisories.");
    } else {
        eprintln!("No security vulnerability advisories found.");
    }

    Ok(())
}

fn colorize_severity(severity: Option<&str>) -> colored::ColoredString {
    match severity {
        Some("critical") => "critical".red().bold(),
        Some("high") => "high".red(),
        Some("medium") => "medium".yellow(),
        Some("low") => "low".blue(),
        _ => "unknown".normal(),
    }
}

//! Audit command - check installed packages for security vulnerabilities.

use anyhow::{Context, Result};
use clap::Args;
use console::style;
use std::collections::HashMap;
use std::path::PathBuf;

use phpx_pm::json::ComposerLock;

#[derive(Args, Debug)]
pub struct AuditArgs {
    /// Only audit packages from composer.lock (not vendor)
    #[arg(long)]
    pub locked: bool,

    /// Skip dev dependencies
    #[arg(long)]
    pub no_dev: bool,

    /// Output as JSON
    #[arg(long)]
    pub format_json: bool,

    /// Working directory
    #[arg(short = 'd', long, default_value = ".")]
    pub working_dir: PathBuf,
}

/// Security advisory from Packagist
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityAdvisory {
    pub advisory_id: String,
    pub package_name: String,
    pub affected_versions: String,
    pub title: String,
    #[serde(default)]
    pub cve: Option<String>,
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default)]
    pub reported_at: Option<String>,
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub sources: Vec<AdvisorySource>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvisorySource {
    pub name: String,
    #[serde(default)]
    pub remote_id: Option<String>,
}

/// Response from Packagist security advisories API
#[derive(Debug, serde::Deserialize)]
struct AdvisoriesResponse {
    advisories: HashMap<String, Vec<SecurityAdvisory>>,
}

/// Audit result for output
#[derive(Debug, serde::Serialize)]
pub struct AuditResult {
    pub advisories: Vec<SecurityAdvisory>,
    pub packages_checked: usize,
    pub vulnerabilities_found: usize,
}

pub async fn execute(args: AuditArgs) -> Result<i32> {
    let working_dir = args.working_dir.canonicalize()
        .context("Failed to resolve working directory")?;

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

    // Collect package names to audit
    let mut packages: Vec<(&str, &str)> = composer_lock.packages.iter()
        .filter(|p| !is_platform_package(&p.name))
        .map(|p| (p.name.as_str(), p.version.as_str()))
        .collect();

    if !args.no_dev {
        packages.extend(
            composer_lock.packages_dev.iter()
                .filter(|p| !is_platform_package(&p.name))
                .map(|p| (p.name.as_str(), p.version.as_str()))
        );
    }

    if packages.is_empty() {
        println!("{} No packages to audit", style("Info:").cyan());
        return Ok(0);
    }

    println!("{} Checking {} packages for security vulnerabilities...",
        style("Info:").cyan(),
        packages.len()
    );

    // Query Packagist security advisories API
    let advisories = fetch_security_advisories(&packages).await?;

    // Filter advisories to only include those affecting installed versions
    let mut matching_advisories: Vec<SecurityAdvisory> = Vec::new();

    for (pkg_name, pkg_version) in &packages {
        if let Some(pkg_advisories) = advisories.get(*pkg_name) {
            for advisory in pkg_advisories {
                if version_matches_constraint(pkg_version, &advisory.affected_versions) {
                    let mut adv = advisory.clone();
                    adv.package_name = pkg_name.to_string();
                    matching_advisories.push(adv);
                }
            }
        }
    }

    let result = AuditResult {
        packages_checked: packages.len(),
        vulnerabilities_found: matching_advisories.len(),
        advisories: matching_advisories,
    };

    if args.format_json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(if result.vulnerabilities_found > 0 { 1 } else { 0 });
    }

    if result.advisories.is_empty() {
        println!("\n{} No security vulnerability advisories found",
            style("Success:").green().bold()
        );
        return Ok(0);
    }

    // Print advisories
    println!("\n{} Found {} security vulnerability {} affecting {} package(s):\n",
        style("Warning:").yellow().bold(),
        result.advisories.len(),
        if result.advisories.len() == 1 { "advisory" } else { "advisories" },
        result.advisories.iter()
            .map(|a| &a.package_name)
            .collect::<std::collections::HashSet<_>>()
            .len()
    );

    // Group advisories by package
    let mut by_package: HashMap<&str, Vec<&SecurityAdvisory>> = HashMap::new();
    for advisory in &result.advisories {
        by_package.entry(&advisory.package_name)
            .or_default()
            .push(advisory);
    }

    for (pkg_name, pkg_advisories) in by_package {
        // Find installed version for this package
        let installed_version = packages.iter()
            .find(|(name, _)| *name == pkg_name)
            .map(|(_, v)| *v)
            .unwrap_or("unknown");

        println!("{} ({}):",
            style(pkg_name).white().bold(),
            style(installed_version).dim()
        );

        for advisory in pkg_advisories {
            let severity_display = advisory.severity.as_deref().unwrap_or("unknown");
            let severity_styled = match severity_display.to_lowercase().as_str() {
                "critical" => style(severity_display).red().bold(),
                "high" => style(severity_display).red(),
                "medium" => style(severity_display).yellow(),
                "low" => style(severity_display).green(),
                _ => style(severity_display).dim(),
            };

            println!("  {} {}", style("•").dim(), style(&advisory.title).cyan());

            if let Some(ref cve) = advisory.cve {
                println!("    CVE: {}", style(cve).yellow());
            }

            println!("    Severity: {}", severity_styled);
            println!("    Affected versions: {}", style(&advisory.affected_versions).dim());

            if let Some(ref link) = advisory.link {
                println!("    Link: {}", style(link).blue().underlined());
            }

            println!();
        }
    }

    Ok(1)
}

/// Fetch security advisories from Packagist API
async fn fetch_security_advisories(packages: &[(&str, &str)]) -> Result<HashMap<String, Vec<SecurityAdvisory>>> {
    let client = reqwest::Client::new();

    // Build URL with query parameters - Packagist expects packages[]=name format
    let package_params: Vec<String> = packages.iter()
        .map(|(name, _)| format!("packages[]={}", urlencoding::encode(name)))
        .collect();

    let url = format!(
        "https://packagist.org/api/security-advisories/?{}",
        package_params.join("&")
    );

    let response = client
        .get(&url)
        .header("User-Agent", format!("phpx/{}", env!("CARGO_PKG_VERSION")))
        .send()
        .await
        .context("Failed to fetch security advisories from Packagist")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Packagist API returned error {}: {}", status, body);
    }

    let advisories_response: AdvisoriesResponse = response.json().await
        .context("Failed to parse security advisories response")?;

    Ok(advisories_response.advisories)
}

/// Check if a package is a platform package (php, ext-*, lib-*)
fn is_platform_package(name: &str) -> bool {
    name == "php" ||
    name.starts_with("php-") ||
    name.starts_with("ext-") ||
    name.starts_with("lib-")
}

/// Check if an installed version matches the affected versions constraint
/// This is a simplified version matcher - Composer uses a full semver parser
fn version_matches_constraint(installed: &str, constraint: &str) -> bool {
    // Normalize installed version
    let installed = installed.trim_start_matches('v');

    // Parse the installed version
    let installed_parts: Vec<u64> = installed
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|s| s.parse().ok())
        .collect();

    if installed_parts.is_empty() {
        return false;
    }

    // Parse constraint - can be complex like ">=1.0,<1.2.3|>=2.0,<2.1"
    // Split by | for OR conditions
    for or_part in constraint.split('|') {
        let or_part = or_part.trim();
        if or_part.is_empty() {
            continue;
        }

        // Split by , for AND conditions
        let mut all_match = true;
        for and_part in or_part.split(',') {
            let and_part = and_part.trim();
            if and_part.is_empty() {
                continue;
            }

            if !check_single_constraint(&installed_parts, and_part) {
                all_match = false;
                break;
            }
        }

        if all_match {
            return true;
        }
    }

    false
}

/// Check a single constraint like ">=1.0" or "<2.0"
fn check_single_constraint(installed: &[u64], constraint: &str) -> bool {
    let constraint = constraint.trim();

    // Extract operator and version
    let (op, version_str) = if constraint.starts_with(">=") {
        (">=", &constraint[2..])
    } else if constraint.starts_with("<=") {
        ("<=", &constraint[2..])
    } else if constraint.starts_with("!=") {
        ("!=", &constraint[2..])
    } else if constraint.starts_with('>') {
        (">", &constraint[1..])
    } else if constraint.starts_with('<') {
        ("<", &constraint[1..])
    } else if constraint.starts_with('=') {
        ("=", &constraint[1..])
    } else if constraint.starts_with('^') {
        ("^", &constraint[1..])
    } else if constraint.starts_with('~') {
        ("~", &constraint[1..])
    } else {
        // Assume exact match
        ("=", constraint)
    };

    let version_str = version_str.trim().trim_start_matches('v');
    let constraint_parts: Vec<u64> = version_str
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|s| s.parse().ok())
        .collect();

    if constraint_parts.is_empty() {
        return true; // Can't parse, assume it matches to be safe
    }

    // Compare versions
    let cmp = compare_version_parts(installed, &constraint_parts);

    match op {
        ">=" => cmp != std::cmp::Ordering::Less,
        "<=" => cmp != std::cmp::Ordering::Greater,
        ">" => cmp == std::cmp::Ordering::Greater,
        "<" => cmp == std::cmp::Ordering::Less,
        "!=" => cmp != std::cmp::Ordering::Equal,
        "=" => cmp == std::cmp::Ordering::Equal,
        "^" => {
            // Caret: >=version, <next-major (or <next-minor for 0.x)
            if cmp == std::cmp::Ordering::Less {
                return false;
            }
            let major = constraint_parts.first().copied().unwrap_or(0);
            let installed_major = installed.first().copied().unwrap_or(0);
            if major == 0 {
                // For 0.x, caret means same minor
                let minor = constraint_parts.get(1).copied().unwrap_or(0);
                let installed_minor = installed.get(1).copied().unwrap_or(0);
                installed_major == major && installed_minor == minor
            } else {
                installed_major == major
            }
        }
        "~" => {
            // Tilde: >=version, <next-minor
            if cmp == std::cmp::Ordering::Less {
                return false;
            }
            let major = constraint_parts.first().copied().unwrap_or(0);
            let minor = constraint_parts.get(1).copied().unwrap_or(0);
            let installed_major = installed.first().copied().unwrap_or(0);
            let installed_minor = installed.get(1).copied().unwrap_or(0);
            installed_major == major && installed_minor == minor
        }
        _ => true,
    }
}

/// Compare two version part arrays
fn compare_version_parts(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    let max_len = std::cmp::max(a.len(), b.len());
    for i in 0..max_len {
        let a_val = a.get(i).copied().unwrap_or(0);
        let b_val = b.get(i).copied().unwrap_or(0);
        match a_val.cmp(&b_val) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

//! Search command - search for packages on Packagist.

use anyhow::{Context, Result};
use clap::Args;
use console::style;
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// Search terms
    #[arg(required = true)]
    pub tokens: Vec<String>,

    /// Search only in package names
    #[arg(short = 'N', long)]
    pub only_name: bool,

    /// Search only for vendor/organization names
    #[arg(short = 'O', long)]
    pub only_vendor: bool,

    /// Filter by package type
    #[arg(short = 't', long)]
    pub r#type: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub format_json: bool,

    /// Working directory
    #[arg(short = 'd', long, default_value = ".")]
    pub working_dir: PathBuf,
}

/// Search result from Packagist
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SearchResult {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub downloads: Option<u64>,
    #[serde(default)]
    pub favers: Option<u64>,
    #[serde(default)]
    pub abandoned: Option<serde_json::Value>,
}

/// Packagist search API response
#[derive(Debug, serde::Deserialize)]
struct PackagistSearchResponse {
    results: Vec<SearchResult>,
    #[serde(default)]
    total: u64,
}

pub async fn execute(args: SearchArgs) -> Result<i32> {
    let query = args.tokens.join(" ");

    if query.is_empty() {
        eprintln!("{} Please provide a search query",
            style("Error:").red().bold()
        );
        return Ok(1);
    }

    // Build search URL
    let mut url = format!(
        "https://packagist.org/search.json?q={}",
        urlencoding::encode(&query)
    );

    if let Some(ref pkg_type) = args.r#type {
        url.push_str(&format!("&type={}", urlencoding::encode(pkg_type)));
    }

    // Fetch results
    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .header("User-Agent", format!("phpx/{}", env!("CARGO_PKG_VERSION")))
        .send()
        .await
        .context("Failed to search Packagist")?;

    if !response.status().is_success() {
        let status = response.status();
        eprintln!("{} Packagist returned error: {}",
            style("Error:").red().bold(),
            status
        );
        return Ok(1);
    }

    let search_response: PackagistSearchResponse = response.json().await
        .context("Failed to parse search response")?;

    let mut results = search_response.results;

    // Filter by vendor if requested
    if args.only_vendor {
        let mut vendors: Vec<String> = results.iter()
            .filter_map(|r| r.name.split('/').next().map(|s| s.to_string()))
            .collect();
        vendors.sort();
        vendors.dedup();

        if args.format_json {
            println!("{}", serde_json::to_string_pretty(&vendors)?);
        } else {
            for vendor in vendors {
                println!("{}", style(&vendor).green());
            }
        }
        return Ok(0);
    }

    // Filter by name match if requested
    if args.only_name {
        let query_lower = query.to_lowercase();
        results.retain(|r| r.name.to_lowercase().contains(&query_lower));
    }

    if results.is_empty() {
        println!("{} No packages found for '{}'",
            style("Info:").cyan(),
            query
        );
        return Ok(0);
    }

    // Output results
    if args.format_json {
        println!("{}", serde_json::to_string_pretty(&results)?);
        return Ok(0);
    }

    // Calculate column widths
    let max_name_len = results.iter()
        .map(|r| r.name.len())
        .max()
        .unwrap_or(30)
        .min(50);

    // Print results
    println!("{} Found {} package(s) for '{}':\n",
        style("Search:").cyan().bold(),
        results.len(),
        query
    );

    for result in &results {
        let name = &result.name;
        let description = result.description.as_deref().unwrap_or("");

        // Check if abandoned
        let is_abandoned = match &result.abandoned {
            Some(serde_json::Value::Bool(true)) => true,
            Some(serde_json::Value::String(_)) => true,
            _ => false,
        };

        let replacement = match &result.abandoned {
            Some(serde_json::Value::String(s)) if !s.is_empty() => Some(s.as_str()),
            _ => None,
        };

        // Print package line
        if is_abandoned {
            print!("{:<width$} ", style(name).yellow().dim(), width = max_name_len);
            print!("{} ", style("! Abandoned !").red());
        } else {
            print!("{:<width$} ", style(name).green().bold(), width = max_name_len);
        }

        // Truncate description if too long
        let term_width = terminal_size::terminal_size()
            .map(|(w, _)| w.0 as usize)
            .unwrap_or(120);
        let available_width = term_width.saturating_sub(max_name_len + 15);

        if description.len() > available_width && available_width > 3 {
            println!("{}...", &description[..available_width - 3]);
        } else {
            println!("{}", description);
        }

        // Show replacement if abandoned
        if let Some(repl) = replacement {
            println!("{:>width$} Use {} instead",
                "",
                style(repl).cyan(),
                width = max_name_len
            );
        }
    }

    // Show pagination info
    if search_response.total > results.len() as u64 {
        println!("\n{} Showing {} of {} results",
            style("Note:").dim(),
            results.len(),
            search_response.total
        );
    }

    Ok(0)
}

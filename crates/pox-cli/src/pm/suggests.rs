use anyhow::{Context, Result};
use clap::Args;
use colored::Colorize;
use pox_pm::json::{ComposerJson, ComposerLock};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct SuggestsArgs {
    /// Show suggestions grouped by suggesting package (default)
    #[arg(long)]
    pub by_package: bool,

    /// Show suggestions grouped by suggested package
    #[arg(long)]
    pub by_suggestion: bool,

    /// Show suggestions from all dependencies, including transitive ones
    #[arg(short, long)]
    pub all: bool,

    /// Show only list of suggested package names
    #[arg(long)]
    pub list: bool,

    /// Exclude suggestions from require-dev packages
    #[arg(long)]
    pub no_dev: bool,

    /// Packages to show suggestions from
    #[arg(name = "packages")]
    pub packages: Vec<String>,

    /// Working directory
    #[arg(short = 'd', long, default_value = ".")]
    pub working_dir: PathBuf,
}

#[derive(Debug, Clone)]
struct Suggestion {
    source: String,
    target: String,
    reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum OutputMode {
    List,
    ByPackage,
    BySuggestion,
    Both,
}

pub async fn execute(args: SuggestsArgs) -> Result<i32> {
    let working_dir = args
        .working_dir
        .canonicalize()
        .context("Failed to resolve working directory")?;

    let json_path = working_dir.join("composer.json");
    let composer_json: Option<ComposerJson> = if json_path.exists() {
        let content = std::fs::read_to_string(&json_path)?;
        Some(serde_json::from_str(&content).context("Failed to parse composer.json")?)
    } else {
        None
    };

    let lock_path = working_dir.join("composer.lock");
    let lock: ComposerLock = if lock_path.exists() {
        let content = std::fs::read_to_string(&lock_path)?;
        serde_json::from_str(&content).context("Failed to parse composer.lock")?
    } else {
        return Err(anyhow::anyhow!(
            "No composer.lock found. Run 'install' or 'update' first."
        ));
    };

    let installed_names: HashSet<String> = lock
        .packages
        .iter()
        .chain(lock.packages_dev.iter())
        .map(|p| p.name.to_lowercase())
        .collect();

    let direct_deps: HashSet<String> = composer_json
        .as_ref()
        .map(|json| {
            let mut deps: HashSet<String> = json.require.keys().map(|k| k.to_lowercase()).collect();
            if !args.no_dev {
                deps.extend(json.require_dev.keys().map(|k| k.to_lowercase()));
            }
            deps
        })
        .unwrap_or_default();

    let mut all_suggestions: Vec<Suggestion> = Vec::new();

    if let Some(ref json) = composer_json {
        if let Some(ref name) = json.name {
            if args.packages.is_empty() || args.packages.iter().any(|p| p.eq_ignore_ascii_case(name)) {
                for (target, reason) in &json.suggest {
                    all_suggestions.push(Suggestion {
                        source: name.clone(),
                        target: target.clone(),
                        reason: reason.clone(),
                    });
                }
            }
        }
    }

    let packages_iter = if args.no_dev {
        lock.packages.iter().collect::<Vec<_>>()
    } else {
        lock.packages
            .iter()
            .chain(lock.packages_dev.iter())
            .collect::<Vec<_>>()
    };

    for pkg in packages_iter {
        if !args.packages.is_empty()
            && !args
                .packages
                .iter()
                .any(|p| p.eq_ignore_ascii_case(&pkg.name))
        {
            continue;
        }

        for (target, reason) in &pkg.suggest {
            all_suggestions.push(Suggestion {
                source: pkg.name.clone(),
                target: target.clone(),
                reason: reason.clone(),
            });
        }
    }

    let suggestions: Vec<Suggestion> = all_suggestions
        .into_iter()
        .filter(|s| !installed_names.contains(&s.target.to_lowercase()))
        .collect();

    let (filtered_suggestions, transitive_count) = if args.packages.is_empty() && !args.all {
        let root_name = composer_json
            .as_ref()
            .and_then(|j| j.name.as_ref())
            .map(|n| n.to_lowercase());

        let filtered: Vec<Suggestion> = suggestions
            .iter()
            .filter(|s| {
                let source_lower = s.source.to_lowercase();
                direct_deps.contains(&source_lower)
                    || root_name.as_ref().map_or(false, |r| r == &source_lower)
            })
            .cloned()
            .collect();

        let transitive = suggestions.len() - filtered.len();
        (filtered, transitive)
    } else {
        (suggestions, 0)
    };

    let mode = if args.list {
        OutputMode::List
    } else if args.by_package && args.by_suggestion {
        OutputMode::Both
    } else if args.by_suggestion {
        OutputMode::BySuggestion
    } else {
        OutputMode::ByPackage
    };

    output_suggestions(&filtered_suggestions, mode, transitive_count);

    Ok(0)
}

fn output_suggestions(suggestions: &[Suggestion], mode: OutputMode, transitive_count: usize) {
    if suggestions.is_empty() && transitive_count == 0 {
        return;
    }

    match mode {
        OutputMode::List => {
            let mut targets: Vec<&str> = suggestions.iter().map(|s| s.target.as_str()).collect();
            targets.sort();
            targets.dedup();

            for target in targets {
                println!("{}", target.cyan());
            }
        }
        OutputMode::ByPackage => {
            output_by_package(suggestions);
            output_transitive_hint(transitive_count);
        }
        OutputMode::BySuggestion => {
            output_by_suggestion(suggestions);
            output_transitive_hint(transitive_count);
        }
        OutputMode::Both => {
            output_by_package(suggestions);
            println!("{}", "-".repeat(78).bright_black());
            output_by_suggestion(suggestions);
            output_transitive_hint(transitive_count);
        }
    }
}

fn output_by_package(suggestions: &[Suggestion]) {
    let mut by_source: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();

    for suggestion in suggestions {
        by_source
            .entry(&suggestion.source)
            .or_default()
            .push((&suggestion.target, &suggestion.reason));
    }

    for (source, targets) in by_source {
        println!("{} suggests:", source.yellow());
        for (target, reason) in targets {
            if reason.is_empty() {
                println!(" - {}", target.cyan());
            } else {
                println!(" - {}: {}", target.cyan(), escape_reason(reason));
            }
        }
        println!();
    }
}

fn output_by_suggestion(suggestions: &[Suggestion]) {
    let mut by_target: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();

    for suggestion in suggestions {
        by_target
            .entry(&suggestion.target)
            .or_default()
            .push((&suggestion.source, &suggestion.reason));
    }

    for (target, sources) in by_target {
        println!("{} is suggested by:", target.yellow());
        for (source, reason) in sources {
            if reason.is_empty() {
                println!(" - {}", source.cyan());
            } else {
                println!(" - {}: {}", source.cyan(), escape_reason(reason));
            }
        }
        println!();
    }
}

fn output_transitive_hint(count: usize) {
    if count > 0 {
        let plural = if count == 1 { "" } else { "s" };
        println!(
            "{} additional suggestion{} by transitive dependencies can be shown with {}",
            count.to_string().cyan(),
            plural,
            "--all".cyan()
        );
    }
}

fn escape_reason(reason: &str) -> String {
    reason
        .chars()
        .filter(|c| !c.is_control() || *c == ' ')
        .map(|c| if c == '\n' { ' ' } else { c })
        .collect()
}

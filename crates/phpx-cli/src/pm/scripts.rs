//! Script execution utilities for composer scripts.

use anyhow::{Context, Result};
use console::style;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use phpx_pm::json::ComposerJson;

/// Script execution context to track environment variables
pub struct ScriptContext {
    env_vars: HashMap<String, String>,
}

impl ScriptContext {
    pub fn new() -> Self {
        Self {
            env_vars: HashMap::new(),
        }
    }
}

impl Default for ScriptContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Collect all scripts from composer.json into a map
pub fn collect_scripts(composer_json: &ComposerJson) -> HashMap<&str, Vec<String>> {
    let mut scripts = HashMap::new();

    // Add built-in event scripts
    let events = [
        ("pre-install-cmd", &composer_json.scripts.pre_install_cmd),
        ("post-install-cmd", &composer_json.scripts.post_install_cmd),
        ("pre-update-cmd", &composer_json.scripts.pre_update_cmd),
        ("post-update-cmd", &composer_json.scripts.post_update_cmd),
        ("post-status-cmd", &composer_json.scripts.post_status_cmd),
        ("pre-archive-cmd", &composer_json.scripts.pre_archive_cmd),
        ("post-archive-cmd", &composer_json.scripts.post_archive_cmd),
        ("pre-autoload-dump", &composer_json.scripts.pre_autoload_dump),
        ("post-autoload-dump", &composer_json.scripts.post_autoload_dump),
        ("post-root-package-install", &composer_json.scripts.post_root_package_install),
        ("post-create-project-cmd", &composer_json.scripts.post_create_project_cmd),
        ("pre-operations-exec", &composer_json.scripts.pre_operations_exec),
    ];

    for (name, value) in events {
        let cmds = value.as_vec();
        if !cmds.is_empty() {
            scripts.insert(name, cmds);
        }
    }

    // Add custom scripts
    for (name, value) in &composer_json.scripts.custom {
        let cmds = value.as_vec();
        if !cmds.is_empty() {
            scripts.insert(name.as_str(), cmds);
        }
    }

    scripts
}

/// Run a specific event script if it exists
/// Returns Ok(0) if script doesn't exist or ran successfully
pub fn run_event_script(
    event_name: &str,
    composer_json: &ComposerJson,
    working_dir: &Path,
    quiet: bool,
) -> Result<i32> {
    let scripts = collect_scripts(composer_json);

    let Some(commands) = scripts.get(event_name) else {
        // No script defined for this event, that's fine
        return Ok(0);
    };

    if !quiet {
        println!("{} Running {} ({} command(s))",
            style(">").green().bold(),
            style(event_name).cyan(),
            commands.len()
        );
    }

    let mut ctx = ScriptContext::new();

    for cmd in commands {
        if !quiet {
            println!("{} {}", style(">").green(), style(cmd).dim());
        }

        let exit_code = run_command(cmd, working_dir, &[], &scripts, &mut ctx)?;

        if exit_code != 0 {
            eprintln!("{} Script '{}' returned exit code {}",
                style("Error:").red().bold(),
                event_name,
                exit_code
            );
            return Ok(exit_code);
        }
    }

    Ok(0)
}

/// Run a named script with optional arguments
pub fn run_script(
    script_name: &str,
    composer_json: &ComposerJson,
    working_dir: &Path,
    args: &[String],
) -> Result<i32> {
    let scripts = collect_scripts(composer_json);

    let Some(commands) = scripts.get(script_name) else {
        eprintln!("{} Script '{}' is not defined in this package",
            style("Error:").red().bold(),
            script_name
        );
        eprintln!();
        eprintln!("Available scripts:");
        for name in scripts.keys() {
            eprintln!("  - {}", name);
        }
        return Ok(1);
    };

    println!("{} Running {} ({} command(s))",
        style(">").green().bold(),
        style(script_name).cyan(),
        commands.len()
    );

    let mut ctx = ScriptContext::new();

    for cmd in commands {
        println!("{} {}", style(">").green(), style(cmd).dim());

        let exit_code = run_command(cmd, working_dir, args, &scripts, &mut ctx)?;

        if exit_code != 0 {
            eprintln!("{} Script '{}' returned exit code {}",
                style("Error:").red().bold(),
                script_name,
                exit_code
            );
            return Ok(exit_code);
        }
    }

    Ok(0)
}

/// Run a single command, handling special prefixes
pub fn run_command(
    cmd: &str,
    working_dir: &Path,
    extra_args: &[String],
    scripts: &HashMap<&str, Vec<String>>,
    ctx: &mut ScriptContext,
) -> Result<i32> {
    // Handle @putenv - set environment variable
    if let Some(env_assignment) = cmd.strip_prefix("@putenv ") {
        if let Some((key, value)) = env_assignment.split_once('=') {
            ctx.env_vars.insert(key.to_string(), value.to_string());
            std::env::set_var(key, value);
        }
        return Ok(0);
    }

    // Handle @php - execute with current PHP binary
    if let Some(php_cmd) = cmd.strip_prefix("@php ") {
        let php_binary = std::env::current_exe()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "php".to_string());

        let full_cmd = if extra_args.is_empty() {
            format!("{} {}", php_binary, php_cmd)
        } else {
            format!("{} {} {}", php_binary, php_cmd, extra_args.join(" "))
        };

        return execute_shell_command(&full_cmd, working_dir, &ctx.env_vars);
    }

    // Handle @composer - execute composer command via phpx
    if let Some(composer_cmd) = cmd.strip_prefix("@composer ") {
        let phpx_binary = std::env::current_exe()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "phpx".to_string());

        let full_cmd = if extra_args.is_empty() {
            format!("{} {}", phpx_binary, composer_cmd)
        } else {
            format!("{} {} {}", phpx_binary, composer_cmd, extra_args.join(" "))
        };

        return execute_shell_command(&full_cmd, working_dir, &ctx.env_vars);
    }

    // Handle @script-name - reference to another script
    if let Some(script_ref) = cmd.strip_prefix('@') {
        // Check if this references another script
        if let Some(ref_commands) = scripts.get(script_ref) {
            println!("{} Running referenced script: {}", style(">").green(), style(script_ref).cyan());
            for ref_cmd in ref_commands {
                println!("{} {}", style(">").green(), style(ref_cmd).dim());
                let exit_code = run_command(ref_cmd, working_dir, extra_args, scripts, ctx)?;
                if exit_code != 0 {
                    return Ok(exit_code);
                }
            }
            return Ok(0);
        } else {
            eprintln!("{} Referenced script '{}' not found",
                style("Warning:").yellow(),
                script_ref
            );
            return Ok(1);
        }
    }

    // Regular shell command
    let full_cmd = if extra_args.is_empty() {
        cmd.to_string()
    } else {
        format!("{} {}", cmd, extra_args.join(" "))
    };

    execute_shell_command(&full_cmd, working_dir, &ctx.env_vars)
}

/// Execute a shell command
fn execute_shell_command(cmd: &str, working_dir: &Path, env_vars: &HashMap<String, String>) -> Result<i32> {
    #[cfg(unix)]
    let mut command = Command::new("sh");
    #[cfg(unix)]
    command.arg("-c").arg(cmd);

    #[cfg(windows)]
    let mut command = Command::new("cmd");
    #[cfg(windows)]
    command.arg("/C").arg(cmd);

    command.current_dir(working_dir);

    // Add custom environment variables
    for (key, value) in env_vars {
        command.env(key, value);
    }

    let status = command
        .status()
        .with_context(|| format!("Failed to execute command: {}", cmd))?;

    Ok(status.code().unwrap_or(1))
}

/// List available scripts
pub fn list_scripts(composer_json: &ComposerJson) -> Result<i32> {
    let scripts = collect_scripts(composer_json);

    if scripts.is_empty() {
        println!("{} No scripts defined in composer.json", style("Info:").cyan());
        return Ok(0);
    }

    println!("{}", style("Available scripts:").cyan().bold());
    println!();

    // Separate custom scripts from event scripts
    let mut custom_scripts: Vec<_> = composer_json.scripts.custom.keys().collect();
    custom_scripts.sort();

    let event_scripts = [
        "pre-install-cmd", "post-install-cmd",
        "pre-update-cmd", "post-update-cmd",
        "post-status-cmd",
        "pre-archive-cmd", "post-archive-cmd",
        "pre-autoload-dump", "post-autoload-dump",
        "post-root-package-install",
        "post-create-project-cmd",
        "pre-operations-exec",
    ];

    // Print custom scripts first (these are the user-defined ones)
    if !custom_scripts.is_empty() {
        println!("{}", style("Scripts:").white().bold());
        for name in &custom_scripts {
            if let Some(cmds) = scripts.get(name.as_str()) {
                // Check for description
                let description = composer_json.scripts_descriptions.get(*name);

                if let Some(desc) = description {
                    println!("  {} - {}", style(name).green(), desc);
                } else {
                    println!("  {}", style(name).green());
                }

                for cmd in cmds {
                    println!("    {}", style(cmd).dim());
                }
            }
        }
        println!();
    }

    // Print event scripts (if any are defined)
    let defined_events: Vec<_> = event_scripts.iter()
        .filter(|name| scripts.contains_key(*name))
        .collect();

    if !defined_events.is_empty() {
        println!("{}", style("Event Scripts:").white().bold());
        for name in defined_events {
            if let Some(cmds) = scripts.get(name) {
                println!("  {}", style(name).yellow());
                for cmd in cmds {
                    println!("    {}", style(cmd).dim());
                }
            }
        }
    }

    Ok(0)
}

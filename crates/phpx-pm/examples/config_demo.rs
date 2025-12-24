/// Example demonstrating the Composer configuration system
///
/// This example shows how to:
/// - Create a default configuration
/// - Load configuration from files and environment
/// - Access configuration values
/// - Track configuration sources
///
/// Run with: cargo run --example config_demo

use phpx_pm::config::{Config, ConfigLoader, PreferredInstall, StoreAuths};
use std::env;

fn main() {
    println!("=== Composer Configuration System Demo ===\n");

    // 1. Create default configuration
    println!("1. Default Configuration:");
    let default_config = Config::default();
    println!("   Vendor dir: {:?}", default_config.vendor_dir);
    println!("   Bin dir: {:?}", default_config.bin_dir);
    println!("   Process timeout: {} seconds", default_config.process_timeout);
    println!("   Preferred install: {:?}", default_config.preferred_install);
    println!("   Store auths: {:?}", default_config.store_auths);
    println!("   Secure HTTP: {}", default_config.secure_http);
    println!("   GitHub protocols: {:?}", default_config.github_protocols);
    println!("   GitHub domains: {:?}", default_config.github_domains);
    println!();

    // 2. Create config with base directory
    println!("2. Configuration with Base Directory:");
    let config = Config::with_base_dir("/path/to/project");
    println!("   Base dir: {:?}", config.base_dir());
    println!("   Vendor dir (resolved): {:?}", config.get_vendor_dir());
    println!("   Bin dir (resolved): {:?}", config.get_bin_dir());
    println!();

    // 3. Configuration loader
    println!("3. Configuration Loader:");
    let loader = ConfigLoader::new(true);
    println!("   Composer home: {:?}", loader.get_composer_home());
    println!("   Cache dir: {:?}", loader.get_cache_dir());
    println!();

    // 4. Environment variable handling
    println!("4. Environment Variable Handling:");
    // Set a test environment variable
    env::set_var("COMPOSER_PROCESS_TIMEOUT", "600");
    env::set_var("COMPOSER_VENDOR_DIR", "lib/vendor");

    let loader_with_env = ConfigLoader::new(true);
    if let Some(timeout) = loader_with_env.get_env_u64("process-timeout") {
        println!("   COMPOSER_PROCESS_TIMEOUT: {}", timeout);
    }
    if let Some(vendor_dir) = loader_with_env.get_env_path("vendor-dir") {
        println!("   COMPOSER_VENDOR_DIR: {:?}", vendor_dir);
    }

    // Clean up
    env::remove_var("COMPOSER_PROCESS_TIMEOUT");
    env::remove_var("COMPOSER_VENDOR_DIR");
    println!();

    // 5. Build configuration from all sources
    println!("5. Building Configuration from All Sources:");
    println!("   (This would load from ~/.composer/config.json and composer.json)");

    // Note: We can't actually load project config without a real composer.json
    match Config::build(None::<&str>, true) {
        Ok(config) => {
            println!("   Successfully built configuration");
            println!("   Process timeout: {}", config.process_timeout);
            println!("   Optimize autoloader: {}", config.optimize_autoloader);
            println!("   Sort packages: {}", config.sort_packages);

            // Show configuration sources
            if let Some(source) = config.get_source("process-timeout") {
                println!("   process-timeout source: {}", source.as_str());
            }
            if let Some(source) = config.get_source("vendor-dir") {
                println!("   vendor-dir source: {}", source.as_str());
            }
        }
        Err(e) => {
            println!("   Error building config: {}", e);
        }
    }
    println!();

    // 6. Configuration enums
    println!("6. Configuration Enums:");
    println!("   PreferredInstall::Auto = {:?}", PreferredInstall::Auto);
    println!("   PreferredInstall::Source = {:?}", PreferredInstall::Source);
    println!("   PreferredInstall::Dist = {:?}", PreferredInstall::Dist);
    println!("   StoreAuths::Prompt = {:?}", StoreAuths::Prompt);
    println!("   StoreAuths::True = {:?}", StoreAuths::True);
    println!("   StoreAuths::False = {:?}", StoreAuths::False);
    println!();

    // 7. Platform overrides
    println!("7. Platform Overrides:");
    let mut config = Config::default();
    config.platform.insert("php".to_string(), "8.2.0".to_string());
    config.platform.insert("ext-mbstring".to_string(), "*".to_string());
    println!("   Platform overrides: {:?}", config.platform);
    println!();

    // 8. Authentication configuration
    println!("8. Authentication Configuration:");
    println!("   GitHub OAuth tokens: {} configured", default_config.github_oauth.len());
    println!("   GitLab OAuth tokens: {} configured", default_config.gitlab_oauth.len());
    println!("   HTTP Basic auth: {} configured", default_config.http_basic.len());
    println!("   Bearer tokens: {} configured", default_config.bearer.len());
    println!();

    // 9. Cache configuration
    println!("9. Cache Configuration:");
    let config = Config::build(None::<&str>, true).unwrap_or_default();
    println!("   Cache TTL: {} seconds ({} months)", config.cache_ttl, config.cache_ttl / (30 * 24 * 60 * 60));
    println!("   Cache files TTL: {:?} seconds", config.cache_files_ttl);
    println!("   Cache files max size: {} bytes ({} MB)",
             config.cache_files_maxsize,
             config.cache_files_maxsize / (1024 * 1024));
    println!("   Cache read-only: {}", config.cache_read_only);
    if let Some(cache_dir) = &config.cache_dir {
        println!("   Cache directory: {:?}", cache_dir);
    }
    if let Some(cache_files_dir) = &config.cache_files_dir {
        println!("   Cache files directory: {:?}", cache_files_dir);
    }
    if let Some(cache_repo_dir) = &config.cache_repo_dir {
        println!("   Cache repo directory: {:?}", cache_repo_dir);
    }
    if let Some(cache_vcs_dir) = &config.cache_vcs_dir {
        println!("   Cache VCS directory: {:?}", cache_vcs_dir);
    }
    println!();

    println!("=== Demo Complete ===");
}

//! Symfony Flex plugin - automatic recipe-based configuration.
//!
//! This is a native Rust port of symfony/flex.
//! When symfony/flex is installed, this plugin automatically downloads and applies
//! recipes from the Symfony recipe repositories.
//!
//! Recipes can:
//! - Register bundles in config/bundles.php
//! - Add environment variables to .env files
//! - Add entries to .gitignore
//! - Copy configuration files from the recipe
//!
//! The plugin maintains a symfony.lock file to track installed recipes.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::composer::Composer;
use crate::event::{ComposerEvent, EventListener, EventType, PostAutoloadDumpEvent};
use crate::http::HttpClient;
use crate::package::Package;
use crate::Result;

/// The package name that triggers this plugin.
pub const PACKAGE_NAME: &str = "symfony/flex";

/// Default recipe endpoints.
const DEFAULT_ENDPOINTS: &[&str] = &[
    "https://raw.githubusercontent.com/symfony/recipes/flex/main/index.json",
    "https://raw.githubusercontent.com/symfony/recipes-contrib/flex/main/index.json",
];

/// Symfony Flex plugin - implements EventListener directly.
pub struct SymfonyFlexPlugin;

impl EventListener for SymfonyFlexPlugin {
    fn handle(&self, event: &dyn ComposerEvent, composer: &Composer) -> anyhow::Result<i32> {
        if event.event_type() != EventType::PostAutoloadDump {
            return Ok(0);
        }

        let Some(e) = event.as_any().downcast_ref::<PostAutoloadDumpEvent>() else {
            return Ok(0);
        };

        // Check if our package is installed
        let is_installed = e.packages.iter().any(|p| p.name == PACKAGE_NAME);
        if !is_installed {
            return Ok(0);
        }

        // Run the flex plugin
        let rt = tokio::runtime::Handle::try_current()
            .map(|h| {
                // Already in async context, use it
                h.block_on(self.run_flex(composer, &e.packages))
            })
            .unwrap_or_else(|_| {
                // Not in async context, create a new runtime
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(self.run_flex(composer, &e.packages))
            });

        rt?;
        Ok(0)
    }

    fn priority(&self) -> i32 {
        -10
    }
}

impl SymfonyFlexPlugin {
    async fn run_flex(&self, composer: &Composer, packages: &[Arc<Package>]) -> Result<()> {
        let working_dir = &composer.working_dir;
        let lock_file = working_dir.join("symfony.lock");
        let http_client = &composer.http_client;

        // Load existing lock
        let mut lock = FlexLock::load(&lock_file)?;

        // Get flex configuration from composer.json
        let flex_config = FlexConfig::from_composer_json(&composer.composer_json);

        // Download recipe index
        let index = self
            .download_recipe_index(http_client, &flex_config.endpoints)
            .await?;

        // Find recipes for installed packages
        let mut recipes_to_install = Vec::new();

        for package in packages {
            // Skip if already in lock
            if lock.has(&package.name) {
                continue;
            }

            // Find recipe for this package
            if let Some(recipe) = self.find_recipe(&index, package, http_client).await? {
                recipes_to_install.push(recipe);
            }
        }

        // Apply recipes
        for recipe in &recipes_to_install {
            self.apply_recipe(working_dir, recipe, &flex_config)?;

            // Update lock
            lock.set(&recipe.package_name, recipe.to_lock_data());
        }

        // Save lock file
        lock.save(&lock_file)?;

        Ok(())
    }

    async fn download_recipe_index(
        &self,
        http_client: &HttpClient,
        endpoints: &[String],
    ) -> Result<RecipeIndex> {
        let mut index = RecipeIndex::new();

        for endpoint in endpoints {
            match http_client.get_json::<EndpointIndex>(endpoint).await {
                Ok(endpoint_index) => {
                    // Merge recipes from this endpoint
                    for (package, versions) in endpoint_index.recipes {
                        index.packages.entry(package).or_default().extend(
                            versions.into_iter().map(|v| RecipeVersionInfo {
                                version: v,
                                endpoint: endpoint.clone(),
                                links: endpoint_index.links.clone(),
                                is_contrib: endpoint_index.is_contrib.unwrap_or(false),
                                branch: endpoint_index.branch.clone().unwrap_or_else(|| "main".to_string()),
                                repository: endpoint_index.links.repository.clone(),
                            }),
                        );
                    }
                }
                Err(e) => {
                    // Log warning but continue with other endpoints
                    eprintln!("Warning: Failed to download recipe index from {}: {}", endpoint, e);
                }
            }
        }

        Ok(index)
    }

    async fn find_recipe(
        &self,
        index: &RecipeIndex,
        package: &Package,
        http_client: &HttpClient,
    ) -> Result<Option<Recipe>> {
        let Some(versions) = index.packages.get(&package.name) else {
            return Ok(None);
        };

        // Parse package version
        let pkg_version = parse_version(&package.version);

        // Find best matching recipe version
        let best_match = versions
            .iter()
            .filter(|v| {
                let recipe_version = parse_version(&v.version);
                compare_versions(&pkg_version, &recipe_version) != std::cmp::Ordering::Less
            })
            .max_by(|a, b| {
                let va = parse_version(&a.version);
                let vb = parse_version(&b.version);
                compare_versions(&va, &vb)
            });

        let Some(version_info) = best_match else {
            return Ok(None);
        };

        // Download recipe manifest
        let recipe_url = self.build_recipe_url(&package.name, &version_info.version, &version_info.links);

        match http_client.get_json::<RecipeManifest>(&recipe_url).await {
            Ok(manifest) => {
                Ok(Some(Recipe {
                    package_name: package.name.clone(),
                    version: version_info.version.clone(),
                    manifest,
                    origin: format!(
                        "{}:{}@{}/{}:{}",
                        package.name,
                        version_info.version,
                        version_info.repository,
                        package.name,
                        version_info.branch
                    ),
                    is_contrib: version_info.is_contrib,
                    recipe_ref: None, // Will be set from manifest if available
                }))
            }
            Err(e) => {
                eprintln!("Warning: Failed to download recipe for {}: {}", package.name, e);
                Ok(None)
            }
        }
    }

    fn build_recipe_url(&self, package: &str, version: &str, links: &EndpointLinks) -> String {
        if let Some(template) = &links.recipe_template {
            template
                .replace("{package_dotted}", &package.replace('/', "."))
                .replace("{package}", package)
                .replace("{version}", version)
        } else {
            // Fallback URL construction
            format!(
                "https://raw.githubusercontent.com/symfony/recipes/flex/main/{}/{}/manifest.json",
                package, version
            )
        }
    }

    fn apply_recipe(&self, working_dir: &Path, recipe: &Recipe, config: &FlexConfig) -> Result<()> {
        let manifest = &recipe.manifest;

        // Apply bundles
        if let Some(bundles) = &manifest.bundles {
            self.configure_bundles(working_dir, &recipe.package_name, bundles, config)?;
        }

        // Apply environment variables
        if let Some(env) = &manifest.env {
            self.configure_env(working_dir, &recipe.package_name, env)?;
        }

        // Apply gitignore
        if let Some(gitignore) = &manifest.gitignore {
            self.configure_gitignore(working_dir, &recipe.package_name, gitignore, config)?;
        }

        // Copy files from recipe
        if let Some(copy_from_recipe) = &manifest.copy_from_recipe {
            if let Some(files) = &manifest.files {
                self.copy_from_recipe(working_dir, copy_from_recipe, files, config)?;
            }
        }

        Ok(())
    }

    /// Configure bundles in config/bundles.php
    fn configure_bundles(
        &self,
        working_dir: &Path,
        package_name: &str,
        bundles: &HashMap<String, Vec<String>>,
        config: &FlexConfig,
    ) -> Result<()> {
        let bundles_file = working_dir.join(&config.config_dir).join("bundles.php");

        // Load existing bundles
        let mut registered = self.load_bundles(&bundles_file)?;

        // Add new bundles
        for (class, envs) in bundles {
            let class = class.trim_start_matches('\\').to_string();

            // Don't override existing entries
            if registered.contains_key(&class) {
                continue;
            }

            let mut env_map = HashMap::new();
            for env in envs {
                env_map.insert(env.clone(), true);
            }
            registered.insert(class, env_map);
        }

        // Write bundles file
        self.write_bundles(&bundles_file, &registered)?;

        println!("  Enabling {} as a Symfony bundle", package_name);
        Ok(())
    }

    fn load_bundles(&self, file: &Path) -> Result<HashMap<String, HashMap<String, bool>>> {
        if !file.exists() {
            return Ok(HashMap::new());
        }

        // Parse existing bundles.php
        // This is a simplified parser - in production we'd need a proper PHP parser
        let content = fs::read_to_string(file)?;
        let mut bundles = HashMap::new();

        // Parse lines like: Symfony\Bundle\FrameworkBundle\FrameworkBundle::class => ['all' => true],
        for line in content.lines() {
            let line = line.trim();
            if line.contains("::class") && line.contains("=>") {
                if let Some((class_part, envs_part)) = line.split_once("::class") {
                    let class = class_part.trim().trim_start_matches('\\').to_string();

                    let mut env_map = HashMap::new();
                    // Parse environments from ['all' => true, 'dev' => true]
                    if let Some(start) = envs_part.find('[') {
                        if let Some(end) = envs_part.rfind(']') {
                            let envs_str = &envs_part[start + 1..end];
                            for part in envs_str.split(',') {
                                let part = part.trim();
                                if let Some((env, val)) = part.split_once("=>") {
                                    let env = env.trim().trim_matches('\'').trim_matches('"').to_string();
                                    let val = val.trim() == "true";
                                    if !env.is_empty() {
                                        env_map.insert(env, val);
                                    }
                                }
                            }
                        }
                    }

                    if !class.is_empty() {
                        bundles.insert(class, env_map);
                    }
                }
            }
        }

        Ok(bundles)
    }

    fn write_bundles(
        &self,
        file: &Path,
        bundles: &HashMap<String, HashMap<String, bool>>,
    ) -> Result<()> {
        // Create parent directory if needed
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut content = String::from("<?php\n\nreturn [\n");

        for (class, envs) in bundles {
            content.push_str(&format!("    {}::class => [", class));

            let mut env_parts = Vec::new();
            for (env, value) in envs {
                let val_str = if *value { "true" } else { "false" };
                env_parts.push(format!("'{}' => {}", env, val_str));
            }
            content.push_str(&env_parts.join(", "));
            content.push_str("],\n");
        }

        content.push_str("];\n");

        fs::write(file, content)?;
        Ok(())
    }

    /// Configure environment variables in .env files
    fn configure_env(
        &self,
        working_dir: &Path,
        package_name: &str,
        env_vars: &HashMap<String, String>,
    ) -> Result<()> {
        let dotenv_files = [".env", ".env.dist"];

        for dotenv_name in &dotenv_files {
            let dotenv_path = working_dir.join(dotenv_name);
            if !dotenv_path.exists() {
                continue;
            }

            // Check if already marked
            let content = fs::read_to_string(&dotenv_path)?;
            if content.contains(&format!("###> {} ###", package_name)) {
                continue;
            }

            // Build env block
            let mut block = format!("\n###> {} ###\n", package_name);
            for (key, value) in env_vars {
                if key.starts_with('#') && key[1..].chars().all(|c| c.is_ascii_digit()) {
                    // Comment line
                    if value.is_empty() {
                        block.push_str("#\n");
                    } else {
                        block.push_str(&format!("# {}\n", value));
                    }
                } else {
                    // Env variable
                    let processed_value = self.process_env_value(value);
                    block.push_str(&format!("{}={}\n", key, processed_value));
                }
            }
            block.push_str(&format!("###< {} ###\n", package_name));

            // Append to file
            let mut new_content = content;
            new_content.push_str(&block);
            fs::write(&dotenv_path, new_content)?;
        }

        println!("  Adding environment variable defaults for {}", package_name);
        Ok(())
    }

    fn process_env_value(&self, value: &str) -> String {
        // Handle %generate(secret)% placeholder
        if value == "%generate(secret)%" {
            return self.generate_secret(16);
        }

        // Handle %generate(secret, N)%
        if value.starts_with("%generate(secret,") && value.ends_with(")%") {
            let inner = &value[17..value.len() - 2];
            if let Ok(len) = inner.trim().parse::<usize>() {
                return self.generate_secret(len);
            }
        }

        // Quote values with special characters
        if value.contains(' ') || value.contains('\t') || value.contains('\n')
            || value.contains('&') || value.contains('!') || value.contains('"') {
            return format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""));
        }

        value.to_string()
    }

    fn generate_secret(&self, len: usize) -> String {
        let mut rng = rand::rng();
        let bytes: Vec<u8> = (0..len).map(|_| rng.random()).collect();
        hex::encode(bytes)
    }

    /// Configure .gitignore entries
    fn configure_gitignore(
        &self,
        working_dir: &Path,
        package_name: &str,
        entries: &[String],
        config: &FlexConfig,
    ) -> Result<()> {
        let gitignore_path = working_dir.join(".gitignore");

        // Check if already marked
        let content = if gitignore_path.exists() {
            fs::read_to_string(&gitignore_path)?
        } else {
            String::new()
        };

        if content.contains(&format!("###> {} ###", package_name)) {
            return Ok(());
        }

        // Build gitignore block
        let mut block = format!("\n###> {} ###\n", package_name);
        for entry in entries {
            // Expand target directories
            let expanded = self.expand_target_dir(entry, config);
            block.push_str(&format!("{}\n", expanded));
        }
        block.push_str(&format!("###< {} ###\n", package_name));

        // Append to file
        let mut new_content = content;
        new_content.push_str(&block);
        fs::write(&gitignore_path, new_content)?;

        println!("  Adding entries to .gitignore for {}", package_name);
        Ok(())
    }

    /// Copy files from recipe
    fn copy_from_recipe(
        &self,
        working_dir: &Path,
        copy_manifest: &HashMap<String, String>,
        files: &HashMap<String, RecipeFile>,
        config: &FlexConfig,
    ) -> Result<()> {
        for (source, target) in copy_manifest {
            let target = self.expand_target_dir(target, config);

            if source.ends_with('/') {
                // Copy directory
                for (file_path, file_data) in files {
                    if file_path.starts_with(source) {
                        let relative = &file_path[source.len()..];
                        let dest = working_dir.join(&target).join(relative);
                        self.write_recipe_file(&dest, file_data)?;
                    }
                }
            } else if let Some(file_data) = files.get(source) {
                // Copy single file
                let dest = working_dir.join(&target);
                self.write_recipe_file(&dest, file_data)?;
            }
        }

        Ok(())
    }

    fn write_recipe_file(&self, dest: &Path, file: &RecipeFile) -> Result<()> {
        // Don't overwrite existing files
        if dest.exists() {
            return Ok(());
        }

        // Create parent directories
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }

        // Decode content
        let content = match &file.contents {
            RecipeFileContents::String(s) => s.clone(),
            RecipeFileContents::Lines(lines) => lines.join("\n"),
            RecipeFileContents::Base64(b64) => {
                use base64::Engine;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                String::from_utf8(bytes)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?
            }
        };

        fs::write(dest, &content)?;

        // Set executable permission if needed
        #[cfg(unix)]
        if file.executable.unwrap_or(false) {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(dest)?.permissions();
            perms.set_mode(perms.mode() | 0o111);
            fs::set_permissions(dest, perms)?;
        }

        println!("  Created {}", dest.display());
        Ok(())
    }

    fn expand_target_dir(&self, path: &str, config: &FlexConfig) -> String {
        path.replace("%CONFIG_DIR%", &config.config_dir)
            .replace("%SRC_DIR%", &config.src_dir)
            .replace("%VAR_DIR%", &config.var_dir)
            .replace("%PUBLIC_DIR%", &config.public_dir)
            .replace("%BIN_DIR%", &config.bin_dir)
    }
}

/// Flex lock file (symfony.lock)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FlexLock {
    #[serde(flatten)]
    packages: HashMap<String, serde_json::Value>,
}

impl FlexLock {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(path)?;
        let lock: FlexLock = serde_json::from_str(&content)?;
        Ok(lock)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if self.packages.is_empty() {
            // Remove empty lock file
            if path.exists() {
                fs::remove_file(path)?;
            }
            return Ok(());
        }

        let content = serde_json::to_string_pretty(&self)?;
        fs::write(path, content)?;
        Ok(())
    }

    pub fn has(&self, name: &str) -> bool {
        self.packages.contains_key(name)
    }

    pub fn get(&self, name: &str) -> Option<&serde_json::Value> {
        self.packages.get(name)
    }

    pub fn set(&mut self, name: &str, data: serde_json::Value) {
        self.packages.insert(name.to_string(), data);
    }

    pub fn remove(&mut self, name: &str) {
        self.packages.remove(name);
    }
}

/// Flex configuration from composer.json extra.symfony
#[derive(Debug, Clone)]
pub struct FlexConfig {
    pub endpoints: Vec<String>,
    pub allow_contrib: bool,
    pub config_dir: String,
    pub src_dir: String,
    pub var_dir: String,
    pub public_dir: String,
    pub bin_dir: String,
}

impl Default for FlexConfig {
    fn default() -> Self {
        Self {
            endpoints: DEFAULT_ENDPOINTS.iter().map(|s| s.to_string()).collect(),
            allow_contrib: false,
            config_dir: "config".to_string(),
            src_dir: "src".to_string(),
            var_dir: "var".to_string(),
            public_dir: "public".to_string(),
            bin_dir: "bin".to_string(),
        }
    }
}

impl FlexConfig {
    pub fn from_composer_json(composer_json: &crate::json::ComposerJson) -> Self {
        let mut config = Self::default();

        if let Some(symfony) = composer_json.extra.get("symfony") {
            if let Some(endpoint) = symfony.get("endpoint") {
                match endpoint {
                    serde_json::Value::String(s) => {
                        if s.contains(".json") || s == "flex://defaults" {
                            let mut endpoints = vec![s.clone()];
                            if s != "flex://defaults" {
                                endpoints.push("flex://defaults".to_string());
                            }
                            // Expand flex://defaults
                            config.endpoints = endpoints
                                .into_iter()
                                .flat_map(|e| {
                                    if e == "flex://defaults" {
                                        DEFAULT_ENDPOINTS.iter().map(|s| s.to_string()).collect()
                                    } else {
                                        vec![e]
                                    }
                                })
                                .collect();
                        }
                    }
                    serde_json::Value::Array(arr) => {
                        config.endpoints = arr
                            .iter()
                            .filter_map(|v| v.as_str())
                            .flat_map(|s| {
                                if s == "flex://defaults" {
                                    DEFAULT_ENDPOINTS.iter().map(|s| s.to_string()).collect()
                                } else {
                                    vec![s.to_string()]
                                }
                            })
                            .collect();
                    }
                    _ => {}
                }
            }

            if let Some(allow_contrib) = symfony.get("allow-contrib").and_then(|v| v.as_bool()) {
                config.allow_contrib = allow_contrib;
            }
        }

        // Read directory configurations
        if let Some(extra) = composer_json.extra.get("config-dir").and_then(|v| v.as_str()) {
            config.config_dir = extra.to_string();
        }
        if let Some(extra) = composer_json.extra.get("src-dir").and_then(|v| v.as_str()) {
            config.src_dir = extra.to_string();
        }
        if let Some(extra) = composer_json.extra.get("var-dir").and_then(|v| v.as_str()) {
            config.var_dir = extra.to_string();
        }
        if let Some(extra) = composer_json.extra.get("public-dir").and_then(|v| v.as_str()) {
            config.public_dir = extra.to_string();
        }

        config
    }
}

/// Recipe index from endpoints
#[derive(Debug, Clone, Default)]
struct RecipeIndex {
    packages: HashMap<String, Vec<RecipeVersionInfo>>,
}

impl RecipeIndex {
    fn new() -> Self {
        Self::default()
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct RecipeVersionInfo {
    version: String,
    endpoint: String,
    links: EndpointLinks,
    is_contrib: bool,
    branch: String,
    repository: String,
}

/// Endpoint index response
#[derive(Debug, Clone, Deserialize)]
struct EndpointIndex {
    recipes: HashMap<String, Vec<String>>,
    #[serde(default, rename = "_links")]
    links: EndpointLinks,
    branch: Option<String>,
    is_contrib: Option<bool>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default, Deserialize)]
struct EndpointLinks {
    #[serde(default)]
    repository: String,
    recipe_template: Option<String>,
    recipe_template_relative: Option<String>,
    origin_template: Option<String>,
}

/// Recipe data
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct Recipe {
    package_name: String,
    version: String,
    manifest: RecipeManifest,
    origin: String,
    is_contrib: bool,
    recipe_ref: Option<String>,
}

impl Recipe {
    fn to_lock_data(&self) -> serde_json::Value {
        serde_json::json!({
            "version": self.version,
            "recipe": {
                "version": self.version,
            }
        })
    }
}

/// Recipe manifest (manifest.json)
#[allow(dead_code)]
#[derive(Debug, Clone, Default, Deserialize)]
struct RecipeManifest {
    bundles: Option<HashMap<String, Vec<String>>>,
    env: Option<HashMap<String, String>>,
    gitignore: Option<Vec<String>>,
    #[serde(rename = "copy-from-recipe")]
    copy_from_recipe: Option<HashMap<String, String>>,
    files: Option<HashMap<String, RecipeFile>>,
    #[serde(rename = "composer-scripts")]
    composer_scripts: Option<HashMap<String, serde_json::Value>>,
    #[serde(rename = "ref")]
    recipe_ref: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RecipeFile {
    contents: RecipeFileContents,
    executable: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RecipeFileContents {
    String(String),
    Lines(Vec<String>),
    Base64(String),
}

/// Parse a version string into comparable parts
fn parse_version(version: &str) -> Vec<u32> {
    // Remove common prefixes
    let version = version
        .trim_start_matches("dev-")
        .trim_start_matches('v')
        .trim_end_matches(".x-dev")
        .trim_end_matches("-dev");

    version
        .split('.')
        .filter_map(|p| p.parse::<u32>().ok())
        .collect()
}

/// Compare two parsed versions
fn compare_versions(a: &[u32], b: &[u32]) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let max_len = a.len().max(b.len());
    for i in 0..max_len {
        let av = a.get(i).unwrap_or(&0);
        let bv = b.get(i).unwrap_or(&0);
        match av.cmp(bv) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_parse_version() {
        assert_eq!(parse_version("1.0.0"), vec![1, 0, 0]);
        assert_eq!(parse_version("v2.3"), vec![2, 3]);
        assert_eq!(parse_version("dev-main"), Vec::<u32>::new());
        assert_eq!(parse_version("1.2.x-dev"), vec![1, 2]);
    }

    #[test]
    fn test_compare_versions() {
        use std::cmp::Ordering;

        assert_eq!(compare_versions(&[1, 0], &[1, 0]), Ordering::Equal);
        assert_eq!(compare_versions(&[1, 1], &[1, 0]), Ordering::Greater);
        assert_eq!(compare_versions(&[1, 0], &[1, 1]), Ordering::Less);
        assert_eq!(compare_versions(&[2, 0], &[1, 9]), Ordering::Greater);
    }

    #[test]
    fn test_flex_lock() {
        let temp = TempDir::new().unwrap();
        let lock_path = temp.path().join("symfony.lock");

        let mut lock = FlexLock::default();
        assert!(!lock.has("symfony/framework-bundle"));

        lock.set("symfony/framework-bundle", serde_json::json!({
            "version": "6.0"
        }));
        assert!(lock.has("symfony/framework-bundle"));

        lock.save(&lock_path).unwrap();
        assert!(lock_path.exists());

        let loaded = FlexLock::load(&lock_path).unwrap();
        assert!(loaded.has("symfony/framework-bundle"));
    }

    #[test]
    fn test_flex_config_default() {
        let config = FlexConfig::default();
        assert_eq!(config.config_dir, "config");
        assert_eq!(config.src_dir, "src");
        assert_eq!(config.var_dir, "var");
        assert_eq!(config.public_dir, "public");
        assert!(!config.allow_contrib);
    }

    #[test]
    fn test_expand_target_dir() {
        let plugin = SymfonyFlexPlugin;
        let config = FlexConfig::default();

        assert_eq!(
            plugin.expand_target_dir("%CONFIG_DIR%/packages", &config),
            "config/packages"
        );
        assert_eq!(
            plugin.expand_target_dir("%SRC_DIR%/Entity", &config),
            "src/Entity"
        );
    }

    #[test]
    fn test_process_env_value() {
        let plugin = SymfonyFlexPlugin;

        // Normal value
        assert_eq!(plugin.process_env_value("simple"), "simple");

        // Value with spaces gets quoted
        assert_eq!(plugin.process_env_value("has spaces"), "\"has spaces\"");

        // Secret generation (check length)
        let secret = plugin.process_env_value("%generate(secret)%");
        assert_eq!(secret.len(), 32); // 16 bytes = 32 hex chars

        let secret16 = plugin.process_env_value("%generate(secret, 8)%");
        assert_eq!(secret16.len(), 16); // 8 bytes = 16 hex chars
    }

    #[test]
    fn test_write_bundles() {
        let temp = TempDir::new().unwrap();
        let bundles_file = temp.path().join("config").join("bundles.php");

        let plugin = SymfonyFlexPlugin;
        let mut bundles = HashMap::new();

        let mut envs = HashMap::new();
        envs.insert("all".to_string(), true);
        bundles.insert("Symfony\\Bundle\\FrameworkBundle\\FrameworkBundle".to_string(), envs);

        plugin.write_bundles(&bundles_file, &bundles).unwrap();

        let content = fs::read_to_string(&bundles_file).unwrap();
        assert!(content.contains("<?php"));
        assert!(content.contains("FrameworkBundle::class"));
        assert!(content.contains("'all' => true"));
    }

    #[test]
    fn test_configure_gitignore() {
        let temp = TempDir::new().unwrap();
        let gitignore = temp.path().join(".gitignore");
        fs::write(&gitignore, "# existing\n").unwrap();

        let plugin = SymfonyFlexPlugin;
        let config = FlexConfig::default();

        plugin
            .configure_gitignore(
                temp.path(),
                "test/package",
                &vec!["/%VAR_DIR%/cache".to_string(), "/.env.local".to_string()],
                &config,
            )
            .unwrap();

        let content = fs::read_to_string(&gitignore).unwrap();
        assert!(content.contains("###> test/package ###"));
        assert!(content.contains("/var/cache"));
        assert!(content.contains("/.env.local"));
        assert!(content.contains("###< test/package ###"));
    }

    #[test]
    fn test_configure_env() {
        let temp = TempDir::new().unwrap();
        let dotenv = temp.path().join(".env");
        fs::write(&dotenv, "# existing\nAPP_ENV=dev\n").unwrap();

        let plugin = SymfonyFlexPlugin;

        let mut env_vars = HashMap::new();
        env_vars.insert("DATABASE_URL".to_string(), "sqlite:///%kernel.project_dir%/var/data.db".to_string());
        env_vars.insert("#1".to_string(), "Database configuration".to_string());

        plugin
            .configure_env(temp.path(), "doctrine/doctrine-bundle", &env_vars)
            .unwrap();

        let content = fs::read_to_string(&dotenv).unwrap();
        assert!(content.contains("###> doctrine/doctrine-bundle ###"));
        assert!(content.contains("DATABASE_URL="));
        assert!(content.contains("# Database configuration"));
        assert!(content.contains("###< doctrine/doctrine-bundle ###"));
    }
}

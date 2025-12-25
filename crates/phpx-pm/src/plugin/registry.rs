//! Plugin registry - manages ported Composer plugins.

use std::path::Path;

use crate::json::ComposerJson;
use crate::package::Package;
use crate::Result;

use super::composer_bin::ComposerBinPlugin;
use super::phpstan_extension_installer::PhpstanExtensionInstallerPlugin;
use super::symfony_runtime::SymfonyRuntimePlugin;

/// Registry of ported Composer plugins.
///
/// Since phpx cannot execute PHP-based Composer plugins, this registry
/// contains native Rust implementations of popular plugins that are
/// activated when the corresponding package is installed.
pub struct PluginRegistry {
    plugins: Vec<Box<dyn ComposerPlugin>>,
}

/// Trait for ported Composer plugins.
pub trait ComposerPlugin: Send + Sync {
    /// The package name that triggers this plugin (e.g., "symfony/runtime").
    fn package_name(&self) -> &str;

    /// Called after the autoload dump is complete.
    fn post_autoload_dump(
        &self,
        vendor_dir: &Path,
        project_dir: &Path,
        composer_json: &ComposerJson,
        installed_packages: &[Package],
    ) -> Result<()>;
}

impl PluginRegistry {
    /// Create a new plugin registry with all available ported plugins.
    pub fn new() -> Self {
        Self {
            plugins: vec![
                Box::new(ComposerBinPlugin),
                Box::new(PhpstanExtensionInstallerPlugin),
                Box::new(SymfonyRuntimePlugin),
            ],
        }
    }

    /// Run post-autoload-dump hooks for all plugins whose packages are installed.
    pub fn run_post_autoload_dump(
        &self,
        vendor_dir: &Path,
        project_dir: &Path,
        composer_json: &ComposerJson,
        installed_packages: &[Package],
    ) -> Result<()> {
        for plugin in &self.plugins {
            // Check if the plugin's package is installed
            let is_installed = installed_packages
                .iter()
                .any(|p| p.name == plugin.package_name());

            if is_installed {
                plugin.post_autoload_dump(vendor_dir, project_dir, composer_json, installed_packages)?;
            }
        }

        Ok(())
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

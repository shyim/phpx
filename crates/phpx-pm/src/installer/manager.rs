//! Installation manager - orchestrates package installation.

use std::path::PathBuf;
use std::sync::Arc;

use crate::downloader::{DownloadConfig, DownloadManager};
use crate::http::HttpClient;
use crate::package::Package;
use crate::solver::{Operation, Transaction};
use crate::Result;

use super::binary::BinaryInstaller;
use super::library::LibraryInstaller;

/// Installation configuration
#[derive(Debug, Clone)]
pub struct InstallConfig {
    /// Vendor directory
    pub vendor_dir: PathBuf,
    /// Bin directory
    pub bin_dir: PathBuf,
    /// Cache directory
    pub cache_dir: PathBuf,
    /// Prefer source over dist
    pub prefer_source: bool,
    /// Prefer dist over source
    pub prefer_dist: bool,
    /// Run in dry-run mode (no actual changes)
    pub dry_run: bool,
    /// Skip dev dependencies
    pub no_dev: bool,
}

impl Default for InstallConfig {
    fn default() -> Self {
        Self {
            vendor_dir: PathBuf::from("vendor"),
            bin_dir: PathBuf::from("vendor/bin"),
            cache_dir: dirs::cache_dir()
                .unwrap_or_else(|| PathBuf::from(".composer"))
                .join("cache"),
            prefer_source: false,
            prefer_dist: true,
            dry_run: false,
            no_dev: false,
        }
    }
}

/// Installation manager
pub struct InstallationManager {
    library_installer: LibraryInstaller,
    binary_installer: BinaryInstaller,
    config: InstallConfig,
}

/// Result of an installation operation
#[derive(Debug)]
pub struct InstallResult {
    /// Packages that were installed
    pub installed: Vec<Package>,
    /// Packages that were updated (from, to)
    pub updated: Vec<(Package, Package)>,
    /// Packages that were removed
    pub removed: Vec<Package>,
    /// Binaries that were linked
    pub binaries: Vec<PathBuf>,
}

impl InstallationManager {
    /// Create a new installation manager
    pub fn new(http_client: Arc<HttpClient>, config: InstallConfig) -> Self {
        let download_config = DownloadConfig {
            vendor_dir: config.vendor_dir.clone(),
            cache_dir: config.cache_dir.clone(),
            prefer_source: config.prefer_source,
            prefer_dist: config.prefer_dist,
        };

        let download_manager = Arc::new(DownloadManager::new(http_client, download_config));

        let library_installer = LibraryInstaller::new(
            download_manager,
            config.vendor_dir.clone(),
        );

        let binary_installer = BinaryInstaller::new(
            config.bin_dir.clone(),
            config.vendor_dir.clone(),
        );

        Self {
            library_installer,
            binary_installer,
            config,
        }
    }

    /// Execute a transaction (install/update/remove packages)
    pub async fn execute(&self, transaction: &Transaction) -> Result<InstallResult> {
        let mut result = InstallResult {
            installed: Vec::new(),
            updated: Vec::new(),
            removed: Vec::new(),
            binaries: Vec::new(),
        };

        if self.config.dry_run {
            // In dry-run mode, just collect what would be done
            for op in &transaction.operations {
                match op {
                    Operation::Install(pkg) => {
                        result.installed.push(pkg.as_ref().clone());
                    }
                    Operation::Update { from, to } => {
                        result.updated.push((from.as_ref().clone(), to.as_ref().clone()));
                    }
                    Operation::Uninstall(pkg) => {
                        result.removed.push(pkg.as_ref().clone());
                    }
                    Operation::MarkUnneeded(_) => {}
                }
            }
            return Ok(result);
        }

        // Create vendor directory
        tokio::fs::create_dir_all(&self.config.vendor_dir).await?;

        // Process operations in order
        for op in &transaction.operations {
            match op {
                Operation::Install(pkg) => {
                    // Skip platform packages (php, ext-*)
                    if pkg.name == "php" || pkg.name.starts_with("ext-") {
                        continue;
                    }
                    let installed = self.install_package(pkg).await?;
                    if installed {
                        let bins = self.binary_installer.install(pkg).await?;
                        result.binaries.extend(bins);
                        result.installed.push(pkg.as_ref().clone());
                    }
                }
                Operation::Update { from, to } => {
                    // Skip platform packages
                    if to.name == "php" || to.name.starts_with("ext-") {
                        continue;
                    }
                    self.update_package(from, to).await?;
                    self.binary_installer.uninstall(from).await?;
                    let bins = self.binary_installer.install(to).await?;
                    result.binaries.extend(bins);
                    result.updated.push((from.as_ref().clone(), to.as_ref().clone()));
                }
                Operation::Uninstall(pkg) => {
                    // Skip platform packages
                    if pkg.name == "php" || pkg.name.starts_with("ext-") {
                        continue;
                    }
                    self.binary_installer.uninstall(pkg).await?;
                    self.uninstall_package(pkg).await?;
                    result.removed.push(pkg.as_ref().clone());
                }
                Operation::MarkUnneeded(_) => {}
            }
        }

        Ok(result)
    }

    /// Install a single package
    /// Returns true if actually installed, false if skipped (already installed)
    async fn install_package(&self, package: &Package) -> Result<bool> {
        let result = self.library_installer.install(package).await?;
        Ok(!result.skipped)
    }

    /// Update a package
    async fn update_package(&self, from: &Package, to: &Package) -> Result<()> {
        self.library_installer.update(from, to).await?;
        Ok(())
    }

    /// Uninstall a package
    async fn uninstall_package(&self, package: &Package) -> Result<()> {
        self.library_installer.uninstall(package).await
    }

    /// Install from a list of packages (without a transaction)
    pub async fn install_packages(&self, packages: &[Package]) -> Result<InstallResult> {
        let mut result = InstallResult {
            installed: Vec::new(),
            updated: Vec::new(),
            removed: Vec::new(),
            binaries: Vec::new(),
        };

        if self.config.dry_run {
            result.installed = packages.to_vec();
            return Ok(result);
        }

        // Create vendor directory
        tokio::fs::create_dir_all(&self.config.vendor_dir).await?;

        for package in packages {
            self.install_package(package).await?;
            let bins = self.binary_installer.install(package).await?;
            result.binaries.extend(bins);
            result.installed.push(package.clone());
        }

        Ok(result)
    }

    /// Get the config
    pub fn config(&self) -> &InstallConfig {
        &self.config
    }
}

/// Helper module for cache directory
mod dirs {
    use std::path::PathBuf;

    pub fn cache_dir() -> Option<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Caches/composer"))
        }

        #[cfg(target_os = "linux")]
        {
            std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
                .map(|p| p.join("composer"))
        }

        #[cfg(target_os = "windows")]
        {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .map(|p| p.join("Composer"))
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".composer/cache"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_install_config_default() {
        let config = InstallConfig::default();
        assert_eq!(config.vendor_dir, PathBuf::from("vendor"));
        assert_eq!(config.bin_dir, PathBuf::from("vendor/bin"));
        assert!(config.prefer_dist);
        assert!(!config.prefer_source);
        assert!(!config.dry_run);
    }

    #[tokio::test]
    async fn test_installation_manager_creation() {
        let http_client = Arc::new(HttpClient::new().unwrap());
        let config = InstallConfig::default();
        let _manager = InstallationManager::new(http_client, config);
    }

    #[tokio::test]
    async fn test_dry_run_install() {
        let http_client = Arc::new(HttpClient::new().unwrap());
        let config = InstallConfig {
            dry_run: true,
            ..Default::default()
        };
        let manager = InstallationManager::new(http_client, config);

        let packages = vec![
            Package::new("vendor/a", "1.0.0"),
            Package::new("vendor/b", "2.0.0"),
        ];

        let result = manager.install_packages(&packages).await.unwrap();
        assert_eq!(result.installed.len(), 2);
        assert!(result.updated.is_empty());
        assert!(result.removed.is_empty());
    }
}

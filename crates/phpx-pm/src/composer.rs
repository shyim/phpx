use std::path::PathBuf;
use std::sync::Arc;
use anyhow::{Context, Result};

use crate::config::Config;
use crate::http::HttpClient;
use crate::json::{ComposerJson, ComposerLock, Repository as JsonRepository, Repositories};
use crate::repository::{ComposerRepository, RepositoryManager};
use crate::installer::InstallationManager;
use crate::installer::InstallConfig;

/// The central Composer application object.
/// 
/// This struct holds the configuration and managers used throughout the application.
pub struct Composer {
    pub config: Config,
    pub composer_json: ComposerJson,
    pub composer_lock: Option<ComposerLock>,
    pub repository_manager: Arc<RepositoryManager>,
    pub installation_manager: Arc<InstallationManager>,
    pub http_client: Arc<HttpClient>,
    pub working_dir: PathBuf,
}

impl Composer {
    /// Create a new Composer instance
    pub fn new(
        working_dir: PathBuf,
        config: Config,
        composer_json: ComposerJson,
        composer_lock: Option<ComposerLock>,
    ) -> Result<Self> {
        let http_client = Arc::new(HttpClient::new().context("Failed to create HTTP client")?);

        // Initialize Repository Manager
        let mut repository_manager = RepositoryManager::new();

        // Check if packagist is disabled in repositories
        let packagist_disabled = is_packagist_disabled(&composer_json.repositories);

        // Add custom repositories from composer.json first (higher priority)
        for repo in composer_json.repositories.as_vec() {
            repository_manager.add_from_json_repository(&repo);
        }

        // Add packagist.org as default repository (unless disabled)
        if !packagist_disabled {
            let packagist = if let Some(cache_dir) = config.cache_dir.clone() {
                ComposerRepository::packagist_with_cache(cache_dir)
            } else {
                ComposerRepository::packagist()
            };
            repository_manager.add_repository(Arc::new(packagist));
        }

        let repository_manager = Arc::new(repository_manager);
        
        
        let install_config = InstallConfig {
            vendor_dir: working_dir.join("vendor"), // Should come from config
            bin_dir: working_dir.join("vendor/bin"), // Should come from config
            cache_dir: config.cache_dir.clone().unwrap_or_else(|| PathBuf::from(".phpx/cache")), // Should leverage Config logic
            prefer_source: false, // Default, can be overridden
            prefer_dist: true,
            dry_run: false,
            no_dev: false,
        };

        let installation_manager = Arc::new(InstallationManager::new(
            http_client.clone(),
            install_config
        ));

        Ok(Self {
            config,
            composer_json,
            composer_lock,
            repository_manager,
            installation_manager,
            http_client,
            working_dir,
        })
    }
}

/// Check if packagist.org is disabled in the repositories configuration
fn is_packagist_disabled(repositories: &Repositories) -> bool {
    match repositories {
        Repositories::None => false,
        Repositories::Array(repos) => {
            // In array format, check for Disabled(false) entries
            // (though this is unusual - disabling is typically done in object format)
            repos.iter().any(|r| matches!(r, JsonRepository::Disabled(false)))
        }
        Repositories::Object(map) => {
            // In object format, packagist.org is disabled if key exists with false value
            map.iter().any(|(key, val)| {
                (key == "packagist.org" || key == "packagist")
                    && matches!(val, JsonRepository::Disabled(false))
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_is_packagist_disabled_none() {
        let repos = Repositories::None;
        assert!(!is_packagist_disabled(&repos));
    }

    #[test]
    fn test_is_packagist_disabled_empty_array() {
        let repos = Repositories::Array(vec![]);
        assert!(!is_packagist_disabled(&repos));
    }

    #[test]
    fn test_is_packagist_disabled_array_with_disabled() {
        let repos = Repositories::Array(vec![JsonRepository::Disabled(false)]);
        assert!(is_packagist_disabled(&repos));
    }

    #[test]
    fn test_is_packagist_disabled_empty_object() {
        let repos = Repositories::Object(HashMap::new());
        assert!(!is_packagist_disabled(&repos));
    }

    #[test]
    fn test_is_packagist_disabled_object_packagist_org_false() {
        let mut map = HashMap::new();
        map.insert("packagist.org".to_string(), JsonRepository::Disabled(false));
        let repos = Repositories::Object(map);
        assert!(is_packagist_disabled(&repos));
    }

    #[test]
    fn test_is_packagist_disabled_object_packagist_false() {
        let mut map = HashMap::new();
        map.insert("packagist".to_string(), JsonRepository::Disabled(false));
        let repos = Repositories::Object(map);
        assert!(is_packagist_disabled(&repos));
    }

    #[test]
    fn test_is_packagist_disabled_object_other_repo() {
        let mut map = HashMap::new();
        map.insert("other-repo".to_string(), JsonRepository::Disabled(false));
        let repos = Repositories::Object(map);
        assert!(!is_packagist_disabled(&repos));
    }
}

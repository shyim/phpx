//! Platform detection for PHP version and extensions.
//!
//! This module detects the installed PHP version and extensions
//! and creates virtual packages that can be used by the dependency solver.

use phpx_pm::Package;

/// Information about the PHP platform
#[derive(Debug, Clone)]
pub struct PlatformInfo {
    /// PHP version string (e.g., "8.4.0")
    pub php_version: String,
    /// PHP version ID (e.g., 80400)
    #[allow(dead_code)]
    pub php_version_id: i32,
    /// List of loaded extensions (lowercase)
    pub extensions: Vec<String>,
}

impl PlatformInfo {
    /// Detect the current PHP platform using the embedded PHP runtime
    pub fn detect() -> Self {
        let version = phpx_embed::Php::version();

        // Get loaded extensions
        let extensions = phpx_embed::Php::get_loaded_extensions()
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.to_lowercase())
            .collect();

        Self {
            php_version: version.version.to_string(),
            php_version_id: version.version_id,
            extensions,
        }
    }

    /// Check if an extension is available
    #[allow(dead_code)]
    pub fn has_extension(&self, name: &str) -> bool {
        let name_lower = name.to_lowercase();
        self.extensions.iter().any(|e| e == &name_lower)
    }

    /// Create virtual packages representing the platform
    ///
    /// Returns packages for:
    /// - `php` with the current PHP version
    /// - `ext-*` for each loaded extension
    pub fn to_packages(&self) -> Vec<Package> {
        let mut packages = Vec::new();

        // PHP version package
        let php_pkg = Package::new("php", &self.php_version);
        packages.push(php_pkg);

        // Extension packages
        for ext in &self.extensions {
            let ext_name = format!("ext-{}", ext);
            // Extensions use the PHP version as their version
            let ext_pkg = Package::new(&ext_name, &self.php_version);
            packages.push(ext_pkg);
        }

        packages
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_detect() {
        let platform = PlatformInfo::detect();
        assert!(!platform.php_version.is_empty());
        assert!(platform.php_version_id > 0);
        // Core is always loaded
        assert!(platform.has_extension("core"));
    }

    #[test]
    fn test_to_packages() {
        let platform = PlatformInfo::detect();
        let packages = platform.to_packages();

        // Should have at least php package
        assert!(!packages.is_empty());
        assert!(packages.iter().any(|p| p.name == "php"));
    }
}

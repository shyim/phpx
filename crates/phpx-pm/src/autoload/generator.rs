//! Autoload generator - creates PHP autoloader files.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use md5::{Md5, Digest};

use crate::package::Autoload;
use crate::Result;

use super::classmap::ClassMapGenerator;

/// Sort packages by dependency weight (topological sort).
/// Packages that are dependencies come first, alphabetical by name as tie-breaker.
fn sort_packages_by_dependency(packages: &[PackageAutoload]) -> Vec<PackageAutoload> {
    if packages.is_empty() {
        return Vec::new();
    }

    // Build a map of package names for quick lookup
    let package_names: HashSet<&str> = packages.iter().map(|p| p.name.as_str()).collect();

    // Calculate weight for each package (number of packages that depend on it)
    let mut weights: HashMap<&str, usize> = HashMap::new();
    for pkg in packages {
        weights.entry(&pkg.name).or_insert(0);
    }

    // For each package, increase weight of its dependencies
    for pkg in packages {
        for dep in &pkg.requires {
            // Only count dependencies that are in our package list
            if package_names.contains(dep.as_str()) {
                *weights.entry(dep.as_str()).or_insert(0) += 1;
            }
        }
    }

    // Sort by weight (descending - most depended-on first), then by name (ascending)
    let mut sorted: Vec<_> = packages.to_vec();
    sorted.sort_by(|a, b| {
        let weight_a = weights.get(a.name.as_str()).unwrap_or(&0);
        let weight_b = weights.get(b.name.as_str()).unwrap_or(&0);

        // Higher weight comes first
        match weight_b.cmp(weight_a) {
            std::cmp::Ordering::Equal => a.name.cmp(&b.name), // Alphabetical tie-breaker
            other => other,
        }
    });

    sorted
}

/// Configuration for autoload generation
#[derive(Debug, Clone)]
pub struct AutoloadConfig {
    /// Vendor directory
    pub vendor_dir: PathBuf,
    /// Base directory (project root)
    pub base_dir: PathBuf,
    /// Whether to optimize autoloader (authoritative classmap)
    pub optimize: bool,
    /// Whether to use APCu for caching
    pub apcu: bool,
    /// Whether to generate authoritative classmap
    pub authoritative: bool,
    /// Suffix for class names (content-hash from lock file)
    pub suffix: Option<String>,
}

impl Default for AutoloadConfig {
    fn default() -> Self {
        Self {
            vendor_dir: PathBuf::from("vendor"),
            base_dir: PathBuf::from("."),
            optimize: false,
            apcu: false,
            authoritative: false,
            suffix: None,
        }
    }
}

/// Package with autoload information for generation
#[derive(Debug, Clone)]
pub struct PackageAutoload {
    /// Package name
    pub name: String,
    /// Autoload configuration
    pub autoload: Autoload,
    /// Install path relative to vendor dir
    pub install_path: String,
    /// Package dependencies (required packages) - used for sorting
    pub requires: Vec<String>,
}

/// Autoload generator
pub struct AutoloadGenerator {
    config: AutoloadConfig,
    classmap_generator: ClassMapGenerator,
}

impl AutoloadGenerator {
    /// Create a new autoload generator
    pub fn new(config: AutoloadConfig) -> Self {
        Self {
            config,
            classmap_generator: ClassMapGenerator::new(),
        }
    }

    /// Get the suffix for class names
    fn get_suffix(&self) -> String {
        self.config.suffix.clone().unwrap_or_else(|| {
            // Generate a random suffix if none provided
            let mut hasher = Md5::new();
            hasher.update(format!("{:?}", std::time::SystemTime::now()).as_bytes());
            format!("{:x}", hasher.finalize())[..16].to_string()
        })
    }

    /// Generate autoloader for installed packages
    pub fn generate(&self, packages: &[PackageAutoload], root_autoload: Option<&Autoload>) -> Result<()> {
        let composer_dir = self.config.vendor_dir.join("composer");
        std::fs::create_dir_all(&composer_dir)?;

        let suffix = self.get_suffix();

        // Sort packages by dependency weight for reproducible output
        let sorted_packages = sort_packages_by_dependency(packages);

        // Collect autoload data from all packages
        // Use BTreeMap for sorted output
        let mut psr4: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut psr0: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut classmap: BTreeMap<String, String> = BTreeMap::new();
        // Files are stored as (identifier, path) pairs - order matters!
        let mut files: Vec<(String, String)> = Vec::new();

        // Process package autoloads in sorted order (dependencies first)
        for pkg in &sorted_packages {
            self.process_autoload(&pkg.autoload, &pkg.install_path, &pkg.name, &mut psr4, &mut psr0, &mut classmap, &mut files)?;
        }

        // Process root autoload last (root overrides)
        if let Some(autoload) = root_autoload {
            self.process_autoload(autoload, "", "__root__", &mut psr4, &mut psr0, &mut classmap, &mut files)?;
        }

        // Generate authoritative classmap if optimizing
        if self.config.optimize || self.config.authoritative {
            self.generate_optimized_classmap(&psr4, &psr0, &mut classmap)?;
        }

        // Add Composer\InstalledVersions to classmap
        classmap.insert(
            "Composer\\InstalledVersions".to_string(),
            "$vendorDir . '/composer/InstalledVersions.php'".to_string(),
        );

        // Generate files
        self.generate_autoload_php(&composer_dir, &suffix)?;
        self.generate_autoload_real(&composer_dir, &suffix, !files.is_empty())?;
        self.generate_autoload_static(&composer_dir, &suffix, &psr4, &psr0, &classmap, &files)?;
        self.generate_autoload_psr4(&composer_dir, &psr4)?;
        self.generate_autoload_namespaces(&composer_dir, &psr0)?;
        self.generate_autoload_classmap(&composer_dir, &classmap)?;
        if !files.is_empty() {
            self.generate_autoload_files(&composer_dir, &files)?;
        }
        self.generate_platform_check(&composer_dir)?;
        self.generate_class_loader(&composer_dir)?;
        self.generate_installed_versions(&composer_dir)?;
        self.generate_installed_php(&composer_dir, &sorted_packages)?;

        Ok(())
    }

    /// Process a package's autoload configuration
    fn process_autoload(
        &self,
        autoload: &Autoload,
        install_path: &str,
        package_name: &str,
        psr4: &mut BTreeMap<String, Vec<String>>,
        psr0: &mut BTreeMap<String, Vec<String>>,
        classmap: &mut BTreeMap<String, String>,
        files: &mut Vec<(String, String)>,
    ) -> Result<()> {
        let is_root = install_path.is_empty();

        // PSR-4
        for (namespace, paths) in &autoload.psr4 {
            // Normalize namespace - strip leading backslash
            let ns = namespace.trim_start_matches('\\').to_string();
            let entry = psr4.entry(ns).or_default();
            for path in paths.as_vec() {
                let full_path = self.get_path_code(install_path, &path, is_root);
                entry.push(full_path);
            }
        }

        // PSR-0
        for (namespace, paths) in &autoload.psr0 {
            let ns = namespace.trim_start_matches('\\').to_string();
            let entry = psr0.entry(ns).or_default();
            for path in paths.as_vec() {
                let full_path = self.get_path_code(install_path, &path, is_root);
                entry.push(full_path);
            }
        }

        // Classmap
        for path in &autoload.classmap {
            let full_path = if is_root {
                self.config.base_dir.join(path)
            } else {
                self.config.vendor_dir.join(install_path).join(path)
            };
            let classes = self.classmap_generator.generate(&full_path)?;
            for (class_name, file_path) in classes {
                let path_code = self.path_to_code(&file_path);
                classmap.insert(class_name, path_code);
            }
        }

        // Files - compute identifier as md5(package_name:path)
        for path in &autoload.files {
            let file_identifier = Self::compute_file_identifier(package_name, path);
            let full_path = self.get_path_code(install_path, path, is_root);
            files.push((file_identifier, full_path));
        }

        Ok(())
    }

    /// Convert a path to PHP code reference ($vendorDir or $baseDir)
    /// This format is used for autoload_psr4.php, autoload_namespaces.php, etc.
    fn get_path_code(&self, install_path: &str, path: &str, is_root: bool) -> String {
        let path = path.trim_end_matches('/');
        if is_root {
            if path.is_empty() || path == "." {
                "$baseDir . '/'".to_string()
            } else {
                format!("$baseDir . '/{}'", path)
            }
        } else {
            let full_path = if path.is_empty() {
                install_path.to_string()
            } else {
                format!("{}/{}", install_path, path)
            };
            format!("$vendorDir . '/{}'", full_path)
        }
    }

    /// Convert an absolute PathBuf to PHP code reference
    fn path_to_code(&self, path: &PathBuf) -> String {
        let path_str = path.to_string_lossy();

        // Check if path is under vendor dir
        let vendor_path = self.config.vendor_dir.canonicalize().unwrap_or_else(|_| self.config.vendor_dir.clone());
        let base_path = self.config.base_dir.canonicalize().unwrap_or_else(|_| self.config.base_dir.clone());

        if let Ok(canonical) = path.canonicalize() {
            if let Ok(rel) = canonical.strip_prefix(&vendor_path) {
                return format!("$vendorDir . '/{}'", rel.display());
            }
            if let Ok(rel) = canonical.strip_prefix(&base_path) {
                return format!("$baseDir . '/{}'", rel.display());
            }
        }

        // Fallback - try without canonicalize
        if let Ok(rel) = path.strip_prefix(&self.config.vendor_dir) {
            return format!("$vendorDir . '/{}'", rel.display());
        }
        if let Ok(rel) = path.strip_prefix(&self.config.base_dir) {
            return format!("$baseDir . '/{}'", rel.display());
        }

        // Last resort - use $baseDir with the path
        format!("$baseDir . '/{}'", path_str)
    }

    /// Generate optimized classmap from PSR-4/PSR-0 directories
    fn generate_optimized_classmap(
        &self,
        psr4: &BTreeMap<String, Vec<String>>,
        psr0: &BTreeMap<String, Vec<String>>,
        classmap: &mut BTreeMap<String, String>,
    ) -> Result<()> {
        // Scan PSR-4 directories
        for paths in psr4.values() {
            for path_code in paths {
                // Extract actual path from code like "$vendorDir . '/symfony/console'"
                if let Some(path) = self.extract_path_from_code(path_code) {
                    let classes = self.classmap_generator.generate(Path::new(&path))?;
                    for (class_name, file_path) in classes {
                        let code = self.path_to_code(&file_path);
                        classmap.insert(class_name, code);
                    }
                }
            }
        }

        // Scan PSR-0 directories
        for paths in psr0.values() {
            for path_code in paths {
                if let Some(path) = self.extract_path_from_code(path_code) {
                    let classes = self.classmap_generator.generate(Path::new(&path))?;
                    for (class_name, file_path) in classes {
                        let code = self.path_to_code(&file_path);
                        classmap.insert(class_name, code);
                    }
                }
            }
        }

        Ok(())
    }

    /// Extract actual filesystem path from PHP code like "$vendorDir . '/path'"
    fn extract_path_from_code(&self, code: &str) -> Option<String> {
        if code.starts_with("$vendorDir") {
            // Extract path after "$vendorDir . '"
            let parts: Vec<&str> = code.splitn(2, "'").collect();
            if parts.len() >= 2 {
                let rel_path = parts[1].trim_end_matches('\'');
                return Some(self.config.vendor_dir.join(rel_path).to_string_lossy().to_string());
            }
        } else if code.starts_with("$baseDir") {
            let parts: Vec<&str> = code.splitn(2, "'").collect();
            if parts.len() >= 2 {
                let rel_path = parts[1].trim_end_matches('\'');
                return Some(self.config.base_dir.join(rel_path).to_string_lossy().to_string());
            }
        }
        None
    }

    /// Generate vendor/autoload.php
    fn generate_autoload_php(&self, _composer_dir: &Path, suffix: &str) -> Result<()> {
        let content = format!(r#"<?php

// autoload.php @generated by Composer

if (PHP_VERSION_ID < 50600) {{
    if (!headers_sent()) {{
        header('HTTP/1.1 500 Internal Server Error');
    }}
    $err = 'Composer 2.3.0 dropped support for autoloading on PHP <5.6 and you are running '.PHP_VERSION.', please upgrade PHP or use Composer 2.2 LTS via "composer self-update --2.2". Aborting.'.PHP_EOL;
    if (!ini_get('display_errors')) {{
        if (PHP_SAPI === 'cli' || PHP_SAPI === 'phpdbg') {{
            fwrite(STDERR, $err);
        }} elseif (!headers_sent()) {{
            echo $err;
        }}
    }}
    throw new RuntimeException($err);
}}

require_once __DIR__ . '/composer/autoload_real.php';

return ComposerAutoloaderInit{suffix}::getLoader();
"#);

        let autoload_path = self.config.vendor_dir.join("autoload.php");
        std::fs::write(autoload_path, content)?;
        Ok(())
    }

    /// Generate vendor/composer/autoload_real.php
    fn generate_autoload_real(&self, composer_dir: &Path, suffix: &str, has_files: bool) -> Result<()> {
        let apcu_prefix = if self.config.apcu {
            format!("        $loader->setApcuPrefix('ComposerAutoloader{}');\n", suffix)
        } else {
            String::new()
        };

        let authoritative = if self.config.authoritative {
            "        $loader->setClassMapAuthoritative(true);\n".to_string()
        } else {
            String::new()
        };

        let files_loader = if has_files {
            format!(r#"
        $filesToLoad = \Composer\Autoload\ComposerStaticInit{suffix}::$files;
        $requireFile = \Closure::bind(static function ($fileIdentifier, $file) {{
            if (empty($GLOBALS['__composer_autoload_files'][$fileIdentifier])) {{
                $GLOBALS['__composer_autoload_files'][$fileIdentifier] = true;

                require $file;
            }}
        }}, null, null);
        foreach ($filesToLoad as $fileIdentifier => $file) {{
            $requireFile($fileIdentifier, $file);
        }}
"#)
        } else {
            String::new()
        };

        let content = format!(r#"<?php

// autoload_real.php @generated by Composer

class ComposerAutoloaderInit{suffix}
{{
    private static $loader;

    public static function loadClassLoader($class)
    {{
        if ('Composer\Autoload\ClassLoader' === $class) {{
            require __DIR__ . '/ClassLoader.php';
        }}
    }}

    /**
     * @return \Composer\Autoload\ClassLoader
     */
    public static function getLoader()
    {{
        if (null !== self::$loader) {{
            return self::$loader;
        }}

        require __DIR__ . '/platform_check.php';

        spl_autoload_register(array('ComposerAutoloaderInit{suffix}', 'loadClassLoader'), true, true);
        self::$loader = $loader = new \Composer\Autoload\ClassLoader(\dirname(__DIR__));
        spl_autoload_unregister(array('ComposerAutoloaderInit{suffix}', 'loadClassLoader'));

        require __DIR__ . '/autoload_static.php';
        call_user_func(\Composer\Autoload\ComposerStaticInit{suffix}::getInitializer($loader));

        $loader->register(true);
{apcu_prefix}{authoritative}{files_loader}
        return $loader;
    }}
}}
"#);

        std::fs::write(composer_dir.join("autoload_real.php"), content)?;
        Ok(())
    }

    /// Convert $vendorDir/$baseDir paths to __DIR__ format for static file
    fn to_static_path(path: &str) -> String {
        if path.starts_with("$vendorDir") {
            // $vendorDir . '/x' => __DIR__ . '/..' . '/x'
            path.replace("$vendorDir", "__DIR__ . '/..'")
        } else if path.starts_with("$baseDir") {
            // $baseDir . '/x' => __DIR__ . '/../..' . '/x'
            path.replace("$baseDir", "__DIR__ . '/../..'")
        } else {
            path.to_string()
        }
    }

    /// Generate vendor/composer/autoload_static.php
    fn generate_autoload_static(
        &self,
        composer_dir: &Path,
        suffix: &str,
        psr4: &BTreeMap<String, Vec<String>>,
        psr0: &BTreeMap<String, Vec<String>>,
        classmap: &BTreeMap<String, String>,
        files: &[(String, String)],
    ) -> Result<()> {
        let mut content = format!(r#"<?php

// autoload_static.php @generated by Composer

namespace Composer\Autoload;

class ComposerStaticInit{suffix}
{{
"#);

        // Generate files array if present
        if !files.is_empty() {
            content.push_str("    public static $files = array (\n");
            for (identifier, path) in files {
                content.push_str(&format!("        '{}' => {},\n", identifier, Self::to_static_path(path)));
            }
            content.push_str("    );\n\n");
        }

        // Generate PSR-4 prefix lengths grouped by first character
        // Sorted in descending order by namespace (krsort equivalent)
        let mut psr4_vec: Vec<_> = psr4.iter().collect();
        psr4_vec.sort_by(|a, b| b.0.cmp(a.0)); // Reverse sort

        if !psr4.is_empty() {
            // Group by first character
            let mut by_first_char: BTreeMap<char, Vec<(&String, usize)>> = BTreeMap::new();
            for (namespace, _) in &psr4_vec {
                let first_char = namespace.chars().next().unwrap_or('_');
                by_first_char.entry(first_char)
                    .or_default()
                    .push((namespace, namespace.len()));
            }

            content.push_str("    public static $prefixLengthsPsr4 = array (\n");
            // Sort by first char descending
            let mut char_entries: Vec<_> = by_first_char.iter().collect();
            char_entries.sort_by(|a, b| b.0.cmp(a.0));

            for (first_char, namespaces) in char_entries {
                content.push_str(&format!("        '{}' =>\n        array (\n", first_char));
                for (ns, len) in namespaces {
                    let ns_escaped = ns.replace('\\', "\\\\");
                    content.push_str(&format!("            '{}' => {},\n", ns_escaped, len));
                }
                content.push_str("        ),\n");
            }
            content.push_str("    );\n\n");

            // Generate PSR-4 prefix directories
            content.push_str("    public static $prefixDirsPsr4 = array (\n");
            for (namespace, paths) in &psr4_vec {
                let ns_escaped = namespace.replace('\\', "\\\\");
                content.push_str(&format!("        '{}' =>\n        array (\n", ns_escaped));
                for (i, path) in paths.iter().enumerate() {
                    content.push_str(&format!("            {} => {},\n", i, Self::to_static_path(path)));
                }
                content.push_str("        ),\n");
            }
            content.push_str("    );\n\n");
        }

        // Generate PSR-0 prefixes if present
        if !psr0.is_empty() {
            let mut psr0_vec: Vec<_> = psr0.iter().collect();
            psr0_vec.sort_by(|a, b| b.0.cmp(a.0));

            // Group by first character
            let mut by_first_char: BTreeMap<char, Vec<(&String, &Vec<String>)>> = BTreeMap::new();
            for (namespace, paths) in &psr0_vec {
                let first_char = namespace.chars().next().unwrap_or('_');
                by_first_char.entry(first_char)
                    .or_default()
                    .push((namespace, paths));
            }

            content.push_str("    public static $prefixesPsr0 = array (\n");
            let mut char_entries: Vec<_> = by_first_char.iter().collect();
            char_entries.sort_by(|a, b| b.0.cmp(a.0));

            for (first_char, namespaces) in char_entries {
                content.push_str(&format!("        '{}' =>\n        array (\n", first_char));
                for (ns, paths) in namespaces {
                    let ns_escaped = ns.replace('\\', "\\\\");
                    content.push_str(&format!("            '{}' =>\n            array (\n", ns_escaped));
                    for (i, path) in paths.iter().enumerate() {
                        content.push_str(&format!("                {} => {},\n", i, Self::to_static_path(path)));
                    }
                    content.push_str("            ),\n");
                }
                content.push_str("        ),\n");
            }
            content.push_str("    );\n\n");
        }

        // Generate classmap
        content.push_str("    public static $classMap = array (\n");
        for (class, path) in classmap {
            let class_escaped = class.replace('\\', "\\\\");
            content.push_str(&format!("        '{}' => {},\n", class_escaped, Self::to_static_path(path)));
        }
        content.push_str("    );\n\n");

        // Generate initializer
        let mut initializer_content = String::new();
        if !psr4.is_empty() {
            initializer_content.push_str(&format!(
                "            $loader->prefixLengthsPsr4 = ComposerStaticInit{}::$prefixLengthsPsr4;\n",
                suffix
            ));
            initializer_content.push_str(&format!(
                "            $loader->prefixDirsPsr4 = ComposerStaticInit{}::$prefixDirsPsr4;\n",
                suffix
            ));
        }
        if !psr0.is_empty() {
            initializer_content.push_str(&format!(
                "            $loader->prefixesPsr0 = ComposerStaticInit{}::$prefixesPsr0;\n",
                suffix
            ));
        }
        initializer_content.push_str(&format!(
            "            $loader->classMap = ComposerStaticInit{}::$classMap;\n",
            suffix
        ));

        content.push_str(&format!(r#"    public static function getInitializer(ClassLoader $loader)
    {{
        return \Closure::bind(function () use ($loader) {{
{}
        }}, null, ClassLoader::class);
    }}
}}
"#, initializer_content));

        std::fs::write(composer_dir.join("autoload_static.php"), content)?;
        Ok(())
    }

    /// Generate vendor/composer/autoload_psr4.php
    fn generate_autoload_psr4(&self, composer_dir: &Path, psr4: &BTreeMap<String, Vec<String>>) -> Result<()> {
        // Sort in descending order like Composer does (krsort)
        let mut psr4_vec: Vec<_> = psr4.iter().collect();
        psr4_vec.sort_by(|a, b| b.0.cmp(a.0));

        let mut entries = Vec::new();
        for (namespace, paths) in psr4_vec {
            let ns_escaped = namespace.replace('\\', "\\\\");
            let paths_str: Vec<String> = paths.iter()
                .map(|p| p.clone())
                .collect();

            entries.push(format!(
                "    '{}' => array({})",
                ns_escaped,
                paths_str.join(", ")
            ));
        }

        let content = format!(r#"<?php

// autoload_psr4.php @generated by Composer

$vendorDir = dirname(__DIR__);
$baseDir = dirname($vendorDir);

return array(
{},
);
"#, entries.join(",\n"));

        std::fs::write(composer_dir.join("autoload_psr4.php"), content)?;
        Ok(())
    }

    /// Generate vendor/composer/autoload_namespaces.php (PSR-0)
    fn generate_autoload_namespaces(&self, composer_dir: &Path, psr0: &BTreeMap<String, Vec<String>>) -> Result<()> {
        let mut psr0_vec: Vec<_> = psr0.iter().collect();
        psr0_vec.sort_by(|a, b| b.0.cmp(a.0));

        let mut entries = Vec::new();
        for (namespace, paths) in psr0_vec {
            let ns_escaped = namespace.replace('\\', "\\\\");
            let paths_str: Vec<String> = paths.iter()
                .map(|p| p.clone())
                .collect();

            entries.push(format!(
                "    '{}' => array({})",
                ns_escaped,
                paths_str.join(", ")
            ));
        }

        let entries_str = if entries.is_empty() {
            String::new()
        } else {
            format!("{},\n", entries.join(",\n"))
        };

        let content = format!(r#"<?php

// autoload_namespaces.php @generated by Composer

$vendorDir = dirname(__DIR__);
$baseDir = dirname($vendorDir);

return array(
{});
"#, entries_str);

        std::fs::write(composer_dir.join("autoload_namespaces.php"), content)?;
        Ok(())
    }

    /// Generate vendor/composer/autoload_classmap.php
    fn generate_autoload_classmap(&self, composer_dir: &Path, classmap: &BTreeMap<String, String>) -> Result<()> {
        let entries: Vec<String> = classmap.iter().map(|(class, path)| {
            format!("    '{}' => {}", class.replace('\\', "\\\\"), path)
        }).collect();

        let entries_str = if entries.is_empty() {
            String::new()
        } else {
            format!("{},\n", entries.join(",\n"))
        };

        let content = format!(r#"<?php

// autoload_classmap.php @generated by Composer

$vendorDir = dirname(__DIR__);
$baseDir = dirname($vendorDir);

return array(
{});
"#, entries_str);

        std::fs::write(composer_dir.join("autoload_classmap.php"), content)?;
        Ok(())
    }

    /// Generate vendor/composer/autoload_files.php
    fn generate_autoload_files(&self, composer_dir: &Path, files: &[(String, String)]) -> Result<()> {
        let entries: Vec<String> = files.iter()
            .map(|(identifier, path)| format!("    '{}' => {}", identifier, path))
            .collect();

        let entries_str = if entries.is_empty() {
            String::new()
        } else {
            format!("{},\n", entries.join(",\n"))
        };

        let content = format!(r#"<?php

// autoload_files.php @generated by Composer

$vendorDir = dirname(__DIR__);
$baseDir = dirname($vendorDir);

return array(
{});
"#, entries_str);

        std::fs::write(composer_dir.join("autoload_files.php"), content)?;
        Ok(())
    }

    /// Generate vendor/composer/platform_check.php
    fn generate_platform_check(&self, composer_dir: &Path) -> Result<()> {
        // Generate a minimal platform check file
        // In a full implementation, this would check PHP version and required extensions
        let content = r#"<?php

// platform_check.php @generated by Composer

$issues = array();

if (!(PHP_VERSION_ID >= 80100)) {
    $issues[] = 'Your Composer dependencies require a PHP version ">= 8.1.0". You are running ' . PHP_VERSION . '.';
}

if ($issues) {
    if (!headers_sent()) {
        header('HTTP/1.1 500 Internal Server Error');
    }
    if (!ini_get('display_errors')) {
        if (PHP_SAPI === 'cli' || PHP_SAPI === 'phpdbg') {
            fwrite(STDERR, 'Composer detected issues in your platform:' . PHP_EOL.PHP_EOL . implode(PHP_EOL, $issues) . PHP_EOL.PHP_EOL);
        } elseif (!headers_sent()) {
            echo 'Composer detected issues in your platform:' . PHP_EOL.PHP_EOL . str_replace('You are running '.PHP_VERSION.'.', '', implode(PHP_EOL, $issues)) . PHP_EOL.PHP_EOL;
        }
    }
    throw new \RuntimeException(
        'Composer detected issues in your platform: ' . implode(' ', $issues)
    );
}
"#;

        std::fs::write(composer_dir.join("platform_check.php"), content)?;
        Ok(())
    }

    /// Generate vendor/composer/InstalledVersions.php
    fn generate_installed_versions(&self, composer_dir: &Path) -> Result<()> {
        // Copy the InstalledVersions.php template
        let content = include_str!("InstalledVersions.php.template");
        std::fs::write(composer_dir.join("InstalledVersions.php"), content)?;
        Ok(())
    }

    /// Generate vendor/composer/ClassLoader.php
    fn generate_class_loader(&self, composer_dir: &Path) -> Result<()> {
        // This is the standard Composer ClassLoader - a simplified version
        let content = include_str!("ClassLoader.php.template");
        std::fs::write(composer_dir.join("ClassLoader.php"), content)?;
        Ok(())
    }

    /// Compute MD5 hash for file identifier (package_name:path)
    /// This matches Composer's behavior
    fn compute_file_identifier(package_name: &str, path: &str) -> String {
        let mut hasher = Md5::new();
        hasher.update(format!("{}:{}", package_name, path).as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Generate vendor/composer/installed.php
    fn generate_installed_php(&self, composer_dir: &Path, packages: &[PackageAutoload]) -> Result<()> {
        let mut package_entries = Vec::new();

        for pkg in packages {
            let entry = format!(r#"        '{}' => array(
            'pretty_version' => 'dev-main',
            'version' => 'dev-main',
            'reference' => null,
            'type' => 'library',
            'install_path' => __DIR__ . '/../{}',
            'aliases' => array(),
            'dev_requirement' => false,
        )"#,
                pkg.name,
                pkg.install_path,
            );
            package_entries.push(entry);
        }

        let content = format!(r#"<?php

// installed.php @generated by Composer

return array(
    'root' => array(
        'name' => '__root__',
        'pretty_version' => 'dev-main',
        'version' => 'dev-main',
        'reference' => null,
        'type' => 'library',
        'install_path' => __DIR__ . '/../../',
        'aliases' => array(),
        'dev' => true,
    ),
    'versions' => array(
{}
    ),
);
"#, package_entries.join(",\n"));

        std::fs::write(composer_dir.join("installed.php"), content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_autoload_config_default() {
        let config = AutoloadConfig::default();
        assert_eq!(config.vendor_dir, PathBuf::from("vendor"));
        assert!(!config.optimize);
        assert!(!config.apcu);
    }

    #[test]
    fn test_generate_empty() {
        let temp_dir = TempDir::new().unwrap();
        let config = AutoloadConfig {
            vendor_dir: temp_dir.path().join("vendor"),
            ..Default::default()
        };

        let generator = AutoloadGenerator::new(config);
        let result = generator.generate(&[], None);

        assert!(result.is_ok());
        assert!(temp_dir.path().join("vendor/autoload.php").exists());
        assert!(temp_dir.path().join("vendor/composer/autoload_real.php").exists());
    }
}

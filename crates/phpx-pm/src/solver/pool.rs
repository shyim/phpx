use std::collections::HashMap;
use std::sync::Arc;
use std::cell::RefCell;

use crate::package::{AliasPackage, Package};
use phpx_semver::{Constraint, ConstraintInterface, Operator, VersionParser};

/// A literal represents a package decision in the SAT solver.
/// Positive literals mean "install package", negative means "don't install".
pub type PackageId = i32;

/// Represents an entry in the pool - either a regular package or an alias
#[derive(Debug, Clone)]
pub enum PoolEntry {
    /// A regular package
    Package(Arc<Package>),
    /// An alias of another package
    Alias(Arc<AliasPackage>),
}

impl PoolEntry {
    /// Returns the package name
    pub fn name(&self) -> &str {
        match self {
            PoolEntry::Package(p) => p.name(),
            PoolEntry::Alias(a) => a.name(),
        }
    }

    /// Returns the version string
    pub fn version(&self) -> &str {
        match self {
            PoolEntry::Package(p) => p.version(),
            PoolEntry::Alias(a) => a.version(),
        }
    }

    /// Returns the pretty version string
    pub fn pretty_version(&self) -> &str {
        match self {
            PoolEntry::Package(p) => p.pretty_version(),
            PoolEntry::Alias(a) => a.pretty_version(),
        }
    }

    /// Returns true if this is an alias package
    pub fn is_alias(&self) -> bool {
        matches!(self, PoolEntry::Alias(_))
    }

    /// Returns the underlying package if this is a Package entry
    /// For aliases, returns None - use as_alias() instead
    pub fn get_package(&self) -> Option<&Arc<Package>> {
        match self {
            PoolEntry::Package(p) => Some(p),
            PoolEntry::Alias(_) => None,
        }
    }

    /// Returns the alias package if this is an alias
    pub fn as_alias(&self) -> Option<&Arc<AliasPackage>> {
        match self {
            PoolEntry::Alias(a) => Some(a),
            _ => None,
        }
    }

    /// Returns the regular package if this is not an alias
    pub fn as_package(&self) -> Option<&Arc<Package>> {
        match self {
            PoolEntry::Package(p) => Some(p),
            _ => None,
        }
    }
}

/// Pool of all available packages for dependency resolution.
///
/// The pool indexes packages by ID (1-based) and by name for efficient lookup.
/// Each package version gets a unique ID that's used as literals in SAT clauses.
pub struct Pool {
    /// All entries indexed by ID (1-based, so index 0 is unused)
    entries: Vec<PoolEntry>,

    /// Legacy: All packages indexed by ID (1-based, so index 0 is unused)
    /// TODO: Remove this once all code uses entries
    packages: Vec<Arc<Package>>,

    /// Package IDs indexed by name (lowercase)
    packages_by_name: HashMap<String, Vec<PackageId>>,

    /// Packages indexed by what they provide (virtual packages)
    providers: HashMap<String, Vec<PackageId>>,

    /// Priority of repositories (lower = higher priority)
    priorities: HashMap<String, i32>,

    /// Repository name for each package (id -> repo name)
    package_repos: HashMap<PackageId, String>,

    /// Cached normalized versions (id -> normalized version)
    normalized_versions: RefCell<HashMap<PackageId, String>>,

    /// Cached parsed constraints (constraint string -> parsed constraint)
    parsed_constraints: RefCell<HashMap<String, Option<Box<dyn ConstraintInterface>>>>,

    /// Maps alias package IDs to their base package IDs
    alias_map: HashMap<PackageId, PackageId>,
}

impl std::fmt::Debug for Pool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pool")
            .field("entries", &self.entries)
            .field("packages_by_name", &self.packages_by_name)
            .field("providers", &self.providers)
            .field("priorities", &self.priorities)
            .field("package_repos", &self.package_repos)
            .field("alias_map", &self.alias_map)
            .finish()
    }
}

impl Pool {
    /// Create a new empty pool
    pub fn new() -> Self {
        let placeholder = Arc::new(Package::new("__placeholder__", "0.0.0"));
        Self {
            entries: vec![PoolEntry::Package(Arc::clone(&placeholder))], // Index 0 placeholder
            packages: vec![placeholder], // Index 0 placeholder
            packages_by_name: HashMap::new(),
            providers: HashMap::new(),
            priorities: HashMap::new(),
            package_repos: HashMap::new(),
            normalized_versions: RefCell::new(HashMap::new()),
            parsed_constraints: RefCell::new(HashMap::new()),
            alias_map: HashMap::new(),
        }
    }

    /// Create a pool builder for fluent construction
    pub fn builder() -> PoolBuilder {
        PoolBuilder::new()
    }

    /// Add a package to the pool, returning its ID
    pub fn add_package(&mut self, package: Package) -> PackageId {
        self.add_package_from_repo(package, None)
    }

    /// Add a package to the pool from a specific repository, returning its ID
    pub fn add_package_from_repo(&mut self, package: Package, repo_name: Option<&str>) -> PackageId {
        let id = self.packages.len() as PackageId;
        let name = package.name.to_lowercase();

        // Index by name
        self.packages_by_name
            .entry(name.clone())
            .or_default()
            .push(id);

        // Index by provides
        for (provided, _constraint) in &package.provide {
            self.providers
                .entry(provided.to_lowercase())
                .or_default()
                .push(id);
        }

        // Index by replaces
        for (replaced, _constraint) in &package.replace {
            self.providers
                .entry(replaced.to_lowercase())
                .or_default()
                .push(id);
        }

        // Track repository source
        if let Some(repo) = repo_name {
            self.package_repos.insert(id, repo.to_string());
        }

        let pkg_arc = Arc::new(package);
        self.entries.push(PoolEntry::Package(Arc::clone(&pkg_arc)));
        self.packages.push(pkg_arc);
        id
    }

    /// Add an alias package to the pool, returning its ID
    ///
    /// This creates a new pool entry for the alias that references the base package.
    /// The alias will have its own ID but share the underlying package data.
    pub fn add_alias(&mut self, alias: AliasPackage) -> PackageId {
        let id = self.entries.len() as PackageId;
        let name = alias.name().to_lowercase();

        // Index by name (so the alias version can be found)
        self.packages_by_name
            .entry(name.clone())
            .or_default()
            .push(id);

        // Index by provides (aliases may have transformed provides)
        for (provided, _constraint) in alias.provide() {
            self.providers
                .entry(provided.to_lowercase())
                .or_default()
                .push(id);
        }

        // Index by replaces
        for (replaced, _constraint) in alias.replace() {
            self.providers
                .entry(replaced.to_lowercase())
                .or_default()
                .push(id);
        }

        // Find the base package ID
        let base_pkg = alias.alias_of();
        let base_id = self.find_package_id(base_pkg.name(), base_pkg.version());

        let alias_arc = Arc::new(alias);
        self.entries.push(PoolEntry::Alias(alias_arc));

        // Also add a placeholder to packages to keep indices in sync
        // (This is a temporary measure until we fully migrate away from packages vec)
        self.packages.push(Arc::new(Package::new("__alias_placeholder__", "0.0.0")));

        // Track alias relationship
        if let Some(base_id) = base_id {
            self.alias_map.insert(id, base_id);
        }

        id
    }

    /// Find a package ID by name and version
    fn find_package_id(&self, name: &str, version: &str) -> Option<PackageId> {
        let name_lower = name.to_lowercase();
        if let Some(ids) = self.packages_by_name.get(&name_lower) {
            for &id in ids {
                if let Some(entry) = self.entry(id) {
                    if entry.version() == version {
                        return Some(id);
                    }
                }
            }
        }
        None
    }

    /// Get an entry by its ID
    pub fn entry(&self, id: PackageId) -> Option<&PoolEntry> {
        if id > 0 && (id as usize) < self.entries.len() {
            Some(&self.entries[id as usize])
        } else {
            None
        }
    }

    /// Check if a package ID represents an alias
    pub fn is_alias(&self, id: PackageId) -> bool {
        self.entry(id).map(|e| e.is_alias()).unwrap_or(false)
    }

    /// Get the base package ID for an alias
    pub fn get_alias_base(&self, id: PackageId) -> Option<PackageId> {
        self.alias_map.get(&id).copied()
    }

    /// Get all aliases for a package
    pub fn get_aliases(&self, base_id: PackageId) -> Vec<PackageId> {
        self.alias_map
            .iter()
            .filter_map(|(&alias_id, &base)| {
                if base == base_id {
                    Some(alias_id)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get a package by its ID
    pub fn package(&self, id: PackageId) -> Option<&Arc<Package>> {
        if id > 0 && (id as usize) < self.packages.len() {
            Some(&self.packages[id as usize])
        } else {
            None
        }
    }

    /// Get all packages with a given name
    pub fn packages_by_name(&self, name: &str) -> Vec<PackageId> {
        self.packages_by_name
            .get(&name.to_lowercase())
            .cloned()
            .unwrap_or_default()
    }

    /// Find all packages that provide a given name (including the name itself)
    ///
    /// This includes:
    /// - Direct matches (packages with the exact name)
    /// - Providers (packages that `provide` this name)
    /// - Replacers (packages that `replace` this name)
    ///
    /// Note: Composer's behavior is that providers/replacers are included in the results,
    /// but the solver will only auto-select them if there's also a direct package available.
    /// If only providers/replacers exist, the user must explicitly require them.
    pub fn what_provides(&self, name: &str, constraint: Option<&str>) -> Vec<PackageId> {
        self.what_provides_with_options(name, constraint, true)
    }

    /// Find only direct packages with the given name (no providers/replacers)
    pub fn what_provides_direct_only(&self, name: &str, constraint: Option<&str>) -> Vec<PackageId> {
        self.what_provides_with_options(name, constraint, false)
    }

    /// Check if there are any direct packages (not just providers/replacers) for a name
    pub fn has_direct_packages(&self, name: &str, constraint: Option<&str>) -> bool {
        !self.what_provides_direct_only(name, constraint).is_empty()
    }

    /// Internal implementation of what_provides with options
    fn what_provides_with_options(&self, name: &str, constraint: Option<&str>, include_providers: bool) -> Vec<PackageId> {
        let name_lower = name.to_lowercase();
        let mut result = Vec::new();

        // Direct matches
        if let Some(ids) = self.packages_by_name.get(&name_lower) {
            for &id in ids {
                if self.matches_constraint(id, constraint) {
                    result.push(id);
                }
            }
        }

        // Providers (provide/replace) - only include if requested
        if include_providers {
            if let Some(ids) = self.providers.get(&name_lower) {
                for &id in ids {
                    // Check if the provider constraint matches
                    // Handle both regular packages and alias packages
                    let provides_version = if let Some(entry) = self.entry(id) {
                        match entry {
                            PoolEntry::Package(pkg) => {
                                pkg.provide.iter()
                                    .find(|(k, _)| k.to_lowercase() == name_lower)
                                    .map(|(_, v)| v.clone())
                                    .or_else(|| {
                                        pkg.replace.iter()
                                            .find(|(k, _)| k.to_lowercase() == name_lower)
                                            .map(|(_, v)| v.clone())
                                    })
                            }
                            PoolEntry::Alias(alias) => {
                                alias.provide().iter()
                                    .find(|(k, _)| k.to_lowercase() == name_lower)
                                    .map(|(_, v)| v.clone())
                                    .or_else(|| {
                                        alias.replace().iter()
                                            .find(|(k, _)| k.to_lowercase() == name_lower)
                                            .map(|(_, v)| v.clone())
                                    })
                            }
                        }
                    } else if let Some(pkg) = self.package(id) {
                        pkg.provide.iter()
                            .find(|(k, _)| k.to_lowercase() == name_lower)
                            .map(|(_, v)| v.clone())
                            .or_else(|| {
                                pkg.replace.iter()
                                    .find(|(k, _)| k.to_lowercase() == name_lower)
                                    .map(|(_, v)| v.clone())
                            })
                    } else {
                        None
                    };

                    if let Some(provided_version) = provides_version {
                        if self.matches_provided_constraint(&provided_version, constraint) {
                            result.push(id);
                        }
                    }
                }
            }
        }

        result
    }

    /// Check if a provided/replaced constraint matches a required constraint.
    ///
    /// In Composer, `provide` and `replace` values are constraints, not versions.
    /// For example, `replace: {"b": ">=1.0"}` means this package can replace B >=1.0.
    /// We need to check if the provide/replace constraint intersects with the require constraint.
    fn matches_provided_constraint(&self, provided_constraint_str: &str, required_constraint: Option<&str>) -> bool {
        let Some(constraint_str) = required_constraint else {
            return true; // No constraint means any version matches
        };

        // Handle wildcard constraints
        if constraint_str == "*" || constraint_str.is_empty() {
            return true;
        }

        // Handle wildcard provided constraints
        if provided_constraint_str == "*" {
            return true;
        }

        let parser = VersionParser::new();

        // Parse the required constraint
        let parsed_required = {
            let constraint_key = constraint_str.to_string();
            let cache = self.parsed_constraints.borrow();
            if let Some(cached) = cache.get(&constraint_key) {
                cached.clone()
            } else {
                drop(cache);
                let parsed = parser.parse_constraints(constraint_str).ok();
                self.parsed_constraints.borrow_mut().insert(constraint_key, parsed.clone());
                parsed
            }
        };

        let Some(parsed_required) = parsed_required else {
            // If constraint parsing fails, accept (be permissive)
            return true;
        };

        // Parse the provided constraint
        let parsed_provided = {
            let constraint_key = provided_constraint_str.to_string();
            let cache = self.parsed_constraints.borrow();
            if let Some(cached) = cache.get(&constraint_key) {
                cached.clone()
            } else {
                drop(cache);
                let parsed = parser.parse_constraints(provided_constraint_str).ok();
                self.parsed_constraints.borrow_mut().insert(constraint_key, parsed.clone());
                parsed
            }
        };

        let Some(parsed_provided) = parsed_provided else {
            // If provided looks like a version (not a constraint), try as exact version
            let normalized_version = match parser.normalize(provided_constraint_str) {
                Ok(v) => v,
                Err(_) => provided_constraint_str.to_string(),
            };

            let version_constraint = match Constraint::new(Operator::Equal, normalized_version) {
                Ok(c) => c,
                Err(_) => return true,
            };

            return parsed_required.matches(&version_constraint);
        };

        // Check if the constraints intersect (have any overlap)
        // Two constraints intersect if they can both be satisfied by some version
        parsed_required.matches(parsed_provided.as_ref())
    }

    /// Check if a package matches a version constraint
    fn matches_constraint(&self, id: PackageId, constraint: Option<&str>) -> bool {
        let Some(constraint_str) = constraint else {
            return true; // No constraint means any version matches
        };

        // Handle wildcard constraints
        if constraint_str == "*" || constraint_str.is_empty() {
            return true;
        }

        // Get the version from either package or alias entry
        let version = if let Some(entry) = self.entry(id) {
            entry.version().to_string()
        } else if let Some(package) = self.package(id) {
            package.version.clone()
        } else {
            return false;
        };

        // Get or compute normalized version (cached)
        let normalized_version = {
            let cache = self.normalized_versions.borrow();
            if let Some(v) = cache.get(&id) {
                v.clone()
            } else {
                drop(cache);
                let parser = VersionParser::new();
                let v = match parser.normalize(&version) {
                    Ok(v) => v,
                    Err(_) => version.clone(),
                };
                self.normalized_versions.borrow_mut().insert(id, v.clone());
                v
            }
        };

        // Get or parse constraint (cached)
        let constraint_key = constraint_str.to_string();
        let parsed_opt = {
            let cache = self.parsed_constraints.borrow();
            if let Some(cached) = cache.get(&constraint_key) {
                cached.clone()
            } else {
                drop(cache);
                let parser = VersionParser::new();
                let parsed = parser.parse_constraints(constraint_str).ok();
                self.parsed_constraints.borrow_mut().insert(constraint_key.clone(), parsed.clone());
                parsed
            }
        };

        let Some(parsed_constraint) = parsed_opt else {
            // If constraint parsing fails, accept all versions (be permissive)
            return true;
        };

        // Create a version constraint (== normalized_version)
        let version_constraint = match Constraint::new(Operator::Equal, normalized_version) {
            Ok(c) => c,
            Err(_) => return true,
        };

        // Check if the version matches the constraint
        parsed_constraint.matches(&version_constraint)
    }

    /// Get the total number of packages (excluding placeholder)
    pub fn len(&self) -> usize {
        self.packages.len() - 1
    }

    /// Check if the pool is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Convert a literal to its package ID (absolute value)
    pub fn literal_to_id(literal: i32) -> PackageId {
        literal.abs()
    }

    /// Check if a literal represents "install" (positive)
    pub fn literal_is_positive(literal: i32) -> bool {
        literal > 0
    }

    /// Create an "install" literal for a package
    pub fn id_to_literal(id: PackageId, install: bool) -> i32 {
        if install { id } else { -id }
    }

    /// Get all package IDs
    pub fn all_package_ids(&self) -> impl Iterator<Item = PackageId> + '_ {
        1..self.packages.len() as PackageId
    }

    /// Set repository priority (lower = higher priority)
    pub fn set_priority(&mut self, repo_name: &str, priority: i32) {
        self.priorities.insert(repo_name.to_string(), priority);
    }

    /// Get priority for a package by its ID
    pub fn get_priority_by_id(&self, id: PackageId) -> i32 {
        if let Some(repo_name) = self.package_repos.get(&id) {
            self.priorities.get(repo_name).copied().unwrap_or(0)
        } else {
            0
        }
    }

    /// Get the repository name for a package
    pub fn get_repository(&self, id: PackageId) -> Option<&str> {
        self.package_repos.get(&id).map(|s| s.as_str())
    }

    /// Get priority for a package's repository (looks up by package name/version)
    pub fn get_priority(&self, package: &Package) -> i32 {
        // Find the package ID by matching name and version
        let name_lower = package.name.to_lowercase();
        if let Some(ids) = self.packages_by_name.get(&name_lower) {
            for &id in ids {
                if let Some(pkg) = self.package(id) {
                    if pkg.version == package.version {
                        return self.get_priority_by_id(id);
                    }
                }
            }
        }
        0
    }
}

impl Default for Pool {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for constructing a Pool with packages from multiple sources
pub struct PoolBuilder {
    pool: Pool,
}

impl PoolBuilder {
    /// Create a new pool builder
    pub fn new() -> Self {
        Self {
            pool: Pool::new(),
        }
    }

    /// Add a package to the pool
    pub fn add_package(mut self, package: Package) -> Self {
        self.pool.add_package(package);
        self
    }

    /// Add a package from a specific repository
    pub fn add_package_from_repo(mut self, package: Package, repo_name: &str) -> Self {
        self.pool.add_package_from_repo(package, Some(repo_name));
        self
    }

    /// Add multiple packages
    pub fn add_packages(mut self, packages: impl IntoIterator<Item = Package>) -> Self {
        for package in packages {
            self.pool.add_package(package);
        }
        self
    }

    /// Add multiple packages from a specific repository
    pub fn add_packages_from_repo(mut self, packages: impl IntoIterator<Item = Package>, repo_name: &str) -> Self {
        for package in packages {
            self.pool.add_package_from_repo(package, Some(repo_name));
        }
        self
    }

    /// Set repository priority
    pub fn set_priority(mut self, repo_name: &str, priority: i32) -> Self {
        self.pool.set_priority(repo_name, priority);
        self
    }

    /// Build the pool
    pub fn build(self) -> Pool {
        self.pool
    }
}

impl Default for PoolBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_add_package() {
        let mut pool = Pool::new();
        let id = pool.add_package(Package::new("vendor/package", "1.0.0"));

        assert_eq!(id, 1);
        assert_eq!(pool.len(), 1);

        let pkg = pool.package(id).unwrap();
        assert_eq!(pkg.name, "vendor/package");
    }

    #[test]
    fn test_pool_packages_by_name() {
        let mut pool = Pool::new();
        pool.add_package(Package::new("vendor/package", "1.0.0"));
        pool.add_package(Package::new("vendor/package", "2.0.0"));
        pool.add_package(Package::new("vendor/other", "1.0.0"));

        let ids = pool.packages_by_name("vendor/package");
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn test_pool_what_provides() {
        let mut pool = Pool::new();

        let mut pkg = Package::new("vendor/impl", "1.0.0");
        pkg.provide.insert("vendor/interface".to_string(), "1.0".to_string());
        pool.add_package(pkg);

        pool.add_package(Package::new("vendor/interface", "1.0.0"));

        let providers = pool.what_provides("vendor/interface", None);
        assert_eq!(providers.len(), 2); // Both the actual package and the provider
    }

    #[test]
    fn test_literal_operations() {
        assert_eq!(Pool::literal_to_id(5), 5);
        assert_eq!(Pool::literal_to_id(-5), 5);
        assert!(Pool::literal_is_positive(5));
        assert!(!Pool::literal_is_positive(-5));
        assert_eq!(Pool::id_to_literal(5, true), 5);
        assert_eq!(Pool::id_to_literal(5, false), -5);
    }

    #[test]
    fn test_pool_builder() {
        let pool = Pool::builder()
            .add_package(Package::new("vendor/a", "1.0.0"))
            .add_package(Package::new("vendor/b", "1.0.0"))
            .build();

        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn test_constraint_matching() {
        let mut pool = Pool::new();
        pool.add_package(Package::new("php", "8.4.0"));
        pool.add_package(Package::new("php", "8.2.0"));
        pool.add_package(Package::new("php", "7.4.0"));

        // Test >=8.4 - should only match 8.4.0
        let matches = pool.what_provides("php", Some(">=8.4"));
        assert_eq!(matches.len(), 1);
        assert_eq!(pool.package(matches[0]).unwrap().version, "8.4.0");

        // Test >=8.0 - should match 8.4.0 and 8.2.0
        let matches = pool.what_provides("php", Some(">=8.0"));
        assert_eq!(matches.len(), 2);

        // Test ^7.4 - should only match 7.4.0
        let matches = pool.what_provides("php", Some("^7.4"));
        assert_eq!(matches.len(), 1);
        assert_eq!(pool.package(matches[0]).unwrap().version, "7.4.0");

        // Test * - should match all
        let matches = pool.what_provides("php", Some("*"));
        assert_eq!(matches.len(), 3);

        // Test None - should match all
        let matches = pool.what_provides("php", None);
        assert_eq!(matches.len(), 3);
    }

    #[test]
    fn test_constraint_matching_semver() {
        let mut pool = Pool::new();
        pool.add_package(Package::new("vendor/pkg", "1.0.0"));
        pool.add_package(Package::new("vendor/pkg", "1.5.0"));
        pool.add_package(Package::new("vendor/pkg", "2.0.0"));

        // Test ^1.0 - should match 1.0.0 and 1.5.0
        let matches = pool.what_provides("vendor/pkg", Some("^1.0"));
        assert_eq!(matches.len(), 2);

        // Test ~1.0 - should match 1.0.0 and 1.5.0
        let matches = pool.what_provides("vendor/pkg", Some("~1.0"));
        assert_eq!(matches.len(), 2);

        // Test >=2.0 - should only match 2.0.0
        let matches = pool.what_provides("vendor/pkg", Some(">=2.0"));
        assert_eq!(matches.len(), 1);

        // Test <2.0 - should match 1.0.0 and 1.5.0
        let matches = pool.what_provides("vendor/pkg", Some("<2.0"));
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn test_provide_constraint_matching() {
        let mut pool = Pool::new();

        // Package that provides psr/log-implementation 1.0
        let mut pkg1 = Package::new("monolog/monolog", "1.0.0");
        pkg1.provide.insert("psr/log-implementation".to_string(), "1.0.0".to_string());
        pool.add_package(pkg1);

        // Package that provides psr/log-implementation 2.0
        let mut pkg2 = Package::new("monolog/monolog", "2.0.0");
        pkg2.provide.insert("psr/log-implementation".to_string(), "2.0.0".to_string());
        pool.add_package(pkg2);

        // Package that provides psr/log-implementation 3.0
        let mut pkg3 = Package::new("monolog/monolog", "3.0.0");
        pkg3.provide.insert("psr/log-implementation".to_string(), "3.0.0".to_string());
        pool.add_package(pkg3);

        // Test ^1.0 - should only match the package providing 1.0
        let matches = pool.what_provides("psr/log-implementation", Some("^1.0"));
        assert_eq!(matches.len(), 1);
        assert_eq!(pool.package(matches[0]).unwrap().version, "1.0.0");

        // Test ^2.0 - should only match the package providing 2.0
        let matches = pool.what_provides("psr/log-implementation", Some("^2.0"));
        assert_eq!(matches.len(), 1);
        assert_eq!(pool.package(matches[0]).unwrap().version, "2.0.0");

        // Test >=2.0 - should match packages providing 2.0 and 3.0
        let matches = pool.what_provides("psr/log-implementation", Some(">=2.0"));
        assert_eq!(matches.len(), 2);

        // Test * - should match all providers
        let matches = pool.what_provides("psr/log-implementation", Some("*"));
        assert_eq!(matches.len(), 3);

        // Test None - should match all providers
        let matches = pool.what_provides("psr/log-implementation", None);
        assert_eq!(matches.len(), 3);
    }

    #[test]
    fn test_provide_wildcard_version() {
        let mut pool = Pool::new();

        // Package that provides with wildcard version (matches any constraint)
        let mut pkg = Package::new("vendor/impl", "1.0.0");
        pkg.provide.insert("vendor/interface".to_string(), "*".to_string());
        pool.add_package(pkg);

        // Should match any constraint
        let matches = pool.what_provides("vendor/interface", Some("^1.0"));
        assert_eq!(matches.len(), 1);

        let matches = pool.what_provides("vendor/interface", Some("^99.0"));
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn test_replace_constraint_matching() {
        let mut pool = Pool::new();

        // Package that replaces another package
        let mut pkg = Package::new("symfony/polyfill-php80", "1.0.0");
        pkg.replace.insert("symfony/polyfill-php73".to_string(), "1.0.0".to_string());
        pool.add_package(pkg);

        // Should find the replacer when looking for replaced package
        let matches = pool.what_provides("symfony/polyfill-php73", Some("^1.0"));
        assert_eq!(matches.len(), 1);

        // Should not match if constraint doesn't match replace version
        let matches = pool.what_provides("symfony/polyfill-php73", Some("^2.0"));
        assert_eq!(matches.len(), 0);
    }

    #[test]
    fn test_pool_add_alias() {
        let mut pool = Pool::new();

        // Add base package
        let base_pkg = Package::new("vendor/package", "dev-main");
        let base_id = pool.add_package(base_pkg.clone());

        // Add alias for the dev-main version
        let alias = AliasPackage::new(
            Arc::new(base_pkg),
            "1.0.0.0".to_string(),
            "1.0.0".to_string(),
        );
        let alias_id = pool.add_alias(alias);

        // Verify alias was added
        assert!(alias_id > base_id);
        assert!(pool.is_alias(alias_id));
        assert!(!pool.is_alias(base_id));

        // Verify alias relationship
        assert_eq!(pool.get_alias_base(alias_id), Some(base_id));
        assert_eq!(pool.get_aliases(base_id), vec![alias_id]);
    }

    #[test]
    fn test_pool_alias_what_provides() {
        let mut pool = Pool::new();

        // Add base package with dev version
        let base_pkg = Package::new("vendor/package", "dev-main");
        pool.add_package(base_pkg.clone());

        // Add alias that makes dev-main appear as 1.0.0
        let alias = AliasPackage::new(
            Arc::new(base_pkg),
            "1.0.0.0".to_string(),
            "1.0.0".to_string(),
        );
        pool.add_alias(alias);

        // Should find both dev-main and the 1.0.0 alias
        let all_versions = pool.packages_by_name("vendor/package");
        assert_eq!(all_versions.len(), 2);

        // Constraint ^1.0 should match the alias
        let matches = pool.what_provides("vendor/package", Some("^1.0"));
        assert_eq!(matches.len(), 1);

        // Constraint dev-main should match the base package
        let matches = pool.what_provides("vendor/package", Some("dev-main"));
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn test_pool_entry_types() {
        let mut pool = Pool::new();

        // Add a regular package
        let pkg = Package::new("vendor/package", "1.0.0");
        let pkg_id = pool.add_package(pkg.clone());

        // Add an alias
        let alias = AliasPackage::new(
            Arc::new(pkg),
            "1.0.x-dev".to_string(),
            "1.0.x-dev".to_string(),
        );
        let alias_id = pool.add_alias(alias);

        // Verify entry types
        let pkg_entry = pool.entry(pkg_id).unwrap();
        assert!(!pkg_entry.is_alias());
        assert!(pkg_entry.as_package().is_some());
        assert!(pkg_entry.as_alias().is_none());

        let alias_entry = pool.entry(alias_id).unwrap();
        assert!(alias_entry.is_alias());
        assert!(alias_entry.as_alias().is_some());
        assert!(alias_entry.get_package().is_none());
    }

    #[test]
    fn test_pool_entry_version() {
        let mut pool = Pool::new();

        // Add base package
        let base_pkg = Package::new("vendor/package", "dev-main");
        let base_id = pool.add_package(base_pkg.clone());

        // Add alias
        let alias = AliasPackage::new(
            Arc::new(base_pkg),
            "2.0.0.0".to_string(),
            "2.0.0".to_string(),
        );
        let alias_id = pool.add_alias(alias);

        // Verify versions through entries
        let base_entry = pool.entry(base_id).unwrap();
        assert_eq!(base_entry.version(), "dev-main");
        assert_eq!(base_entry.name(), "vendor/package");

        let alias_entry = pool.entry(alias_id).unwrap();
        assert_eq!(alias_entry.version(), "2.0.0.0");
        assert_eq!(alias_entry.pretty_version(), "2.0.0");
        assert_eq!(alias_entry.name(), "vendor/package");
    }

}

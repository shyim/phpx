use std::collections::HashSet;

use super::pool::{Pool, PackageId};
use super::request::Request;
use super::rule::{Rule, RuleType};
use super::rule_set::RuleSet;

/// Generates SAT rules from a dependency graph.
///
/// This converts the dependency relationships into SAT clauses:
/// - Root requirements: at least one version must be installed
/// - Package requirements: if A is installed, then B|C|D must be installed
/// - Conflicts: A and B cannot both be installed
/// - Same-name: only one version of a package can be installed
pub struct RuleGenerator<'a> {
    pool: &'a Pool,
    rules: RuleSet,
    /// Packages we've already processed
    added_packages: HashSet<PackageId>,
    /// Package names we've added same-name rules for
    same_name_added: HashSet<String>,
}

impl<'a> RuleGenerator<'a> {
    /// Create a new rule generator
    pub fn new(pool: &'a Pool) -> Self {
        Self {
            pool,
            rules: RuleSet::new(),
            added_packages: HashSet::new(),
            same_name_added: HashSet::new(),
        }
    }

    /// Generate all rules for a request
    pub fn generate(mut self, request: &Request) -> RuleSet {
        // Add fixed package rules first
        self.add_fixed_rules(request);

        // Add root requirement rules
        self.add_root_require_rules(request);

        // Add conflict rules for all processed packages
        self.add_conflict_rules();

        self.rules
    }

    /// Add rules for fixed packages (must be installed)
    fn add_fixed_rules(&mut self, request: &Request) {
        for package in &request.fixed_packages {
            // Find the package in the pool
            let ids = self.pool.packages_by_name(&package.name);
            for id in ids {
                if let Some(pkg) = self.pool.package(id) {
                    if pkg.version == package.version {
                        let rule = Rule::fixed(id)
                            .with_source(id)
                            .with_target(&package.name);
                        self.rules.add(rule);
                        self.add_package_rules(id);
                        break;
                    }
                }
            }
        }
    }

    /// Add rules for root requirements
    fn add_root_require_rules(&mut self, request: &Request) {
        for (name, constraint) in request.all_requires() {
            let providers = self.pool.what_provides(name, Some(constraint));

            if providers.is_empty() {
                // No packages satisfy this requirement
                // Add an empty rule that will cause a conflict
                let rule = Rule::new(vec![], RuleType::RootRequire)
                    .with_target(name)
                    .with_constraint(constraint);
                self.rules.add(rule);
                continue;
            }

            // At least one of the providers must be installed
            let rule = Rule::root_require(providers.clone())
                .with_target(name)
                .with_constraint(constraint);
            self.rules.add(rule);

            // Add dependency rules for each provider
            for id in providers {
                self.add_package_rules(id);
            }
        }
    }

    /// Add all rules for a package (requirements, conflicts, same-name)
    fn add_package_rules(&mut self, package_id: PackageId) {
        if self.added_packages.contains(&package_id) {
            return;
        }
        self.added_packages.insert(package_id);

        let Some(package) = self.pool.package(package_id) else {
            return;
        };

        let package = package.clone();

        // Add same-name rules (only one version can be installed)
        self.add_same_name_rules(&package.name);

        // Add requirement rules
        for (dep_name, constraint) in &package.require {
            // Skip lib-* packages (library constraints like lib-libxml)
            // These are rarely used and hard to detect
            if dep_name.starts_with("lib-") {
                continue;
            }

            let providers = self.pool.what_provides(dep_name, Some(constraint));

            if providers.is_empty() {
                // Dependency cannot be satisfied - if this package is installed, conflict
                let rule = Rule::new(vec![-package_id], RuleType::PackageRequires)
                    .with_source(package_id)
                    .with_target(dep_name)
                    .with_constraint(constraint);
                self.rules.add(rule);
                continue;
            }

            // If package_id is installed, one of providers must be installed
            let rule = Rule::requires(package_id, providers.clone())
                .with_source(package_id)
                .with_target(dep_name)
                .with_constraint(constraint);
            self.rules.add(rule);

            // Recursively process dependencies (skip platform packages)
            for id in providers {
                if let Some(pkg) = self.pool.package(id) {
                    // Platform packages (php, ext-*) don't have dependencies to process
                    if !pkg.name.starts_with("php") && !pkg.name.starts_with("ext-") {
                        self.add_package_rules(id);
                    }
                }
            }
        }

        // Add conflict rules for explicit conflicts
        for (conflict_name, constraint) in &package.conflict {
            let conflicting = self.pool.what_provides(conflict_name, Some(constraint));
            for conflict_id in conflicting {
                if conflict_id != package_id {
                    let rule = Rule::conflict(vec![package_id, conflict_id])
                        .with_source(package_id)
                        .with_target(conflict_name);
                    self.rules.add(rule);
                }
            }
        }
    }

    /// Add same-name rules (only one version of a package can be installed)
    fn add_same_name_rules(&mut self, name: &str) {
        let name_lower = name.to_lowercase();
        if self.same_name_added.contains(&name_lower) {
            return;
        }
        self.same_name_added.insert(name_lower.clone());

        let versions = self.pool.packages_by_name(name);
        if versions.len() <= 1 {
            return;
        }

        // Generate pairwise conflict rules for all versions
        // For efficiency with many versions, we generate (n choose 2) rules
        for i in 0..versions.len() {
            for j in (i + 1)..versions.len() {
                let rule = Rule::conflict(vec![versions[i], versions[j]]);
                self.rules.add(rule);
            }
        }
    }

    /// Add conflict rules for packages that conflict with each other
    fn add_conflict_rules(&mut self) {
        // Collect all conflicts to add
        let mut conflicts: Vec<(PackageId, PackageId)> = Vec::new();

        for &package_id in &self.added_packages {
            let Some(package) = self.pool.package(package_id) else {
                continue;
            };

            // Check replaces - replaced packages conflict with the replacer
            for (replaced_name, _) in &package.replace {
                let replaced_ids = self.pool.packages_by_name(replaced_name);
                for replaced_id in replaced_ids {
                    if replaced_id != package_id {
                        conflicts.push((package_id, replaced_id));
                    }
                }
            }
        }

        // Add conflict rules
        for (a, b) in conflicts {
            let rule = Rule::conflict(vec![a, b]);
            self.rules.add(rule);
        }
    }
}

/// Builder for creating rules with additional context
#[allow(dead_code)]
pub struct RuleBuilder {
    rule: Rule,
}

#[allow(dead_code)]
impl RuleBuilder {
    pub fn new(rule: Rule) -> Self {
        Self { rule }
    }

    pub fn source(mut self, package_id: PackageId) -> Self {
        self.rule = self.rule.with_source(package_id);
        self
    }

    pub fn target(mut self, name: impl Into<String>) -> Self {
        self.rule = self.rule.with_target(name);
        self
    }

    pub fn constraint(mut self, constraint: impl Into<String>) -> Self {
        self.rule = self.rule.with_constraint(constraint);
        self
    }

    pub fn build(self) -> Rule {
        self.rule
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::Package;

    fn create_test_pool() -> Pool {
        let mut pool = Pool::new();

        // Add package A with two versions
        let mut a1 = Package::new("vendor/a", "1.0.0");
        a1.require.insert("vendor/b".to_string(), "^1.0".to_string());
        pool.add_package(a1);

        let mut a2 = Package::new("vendor/a", "2.0.0");
        a2.require.insert("vendor/b".to_string(), "^2.0".to_string());
        pool.add_package(a2);

        // Add package B with two versions
        pool.add_package(Package::new("vendor/b", "1.0.0"));
        pool.add_package(Package::new("vendor/b", "2.0.0"));

        // Add package C that conflicts with B
        let mut c = Package::new("vendor/c", "1.0.0");
        c.conflict.insert("vendor/b".to_string(), "*".to_string());
        pool.add_package(c);

        pool
    }

    #[test]
    fn test_rule_generator_root_require() {
        let pool = create_test_pool();
        let mut request = Request::new();
        request.require("vendor/a", "^1.0");

        let generator = RuleGenerator::new(&pool);
        let rules = generator.generate(&request);

        // Should have root requirement rule
        let root_rules: Vec<_> = rules.rules_of_type(RuleType::RootRequire).collect();
        assert!(!root_rules.is_empty());
    }

    #[test]
    fn test_rule_generator_same_name() {
        let pool = create_test_pool();
        let mut request = Request::new();
        request.require("vendor/a", "*");

        let generator = RuleGenerator::new(&pool);
        let rules = generator.generate(&request);

        // Should have same-name conflict rules for vendor/a versions
        let conflict_rules: Vec<_> = rules.rules_of_type(RuleType::PackageConflict).collect();
        assert!(!conflict_rules.is_empty());
    }

    #[test]
    fn test_rule_generator_package_requires() {
        let pool = create_test_pool();
        let mut request = Request::new();
        request.require("vendor/a", "*");

        let generator = RuleGenerator::new(&pool);
        let rules = generator.generate(&request);

        // Should have package requirement rules
        let require_rules: Vec<_> = rules.rules_of_type(RuleType::PackageRequires).collect();
        assert!(!require_rules.is_empty());
    }

    #[test]
    fn test_rule_generator_fixed() {
        let pool = create_test_pool();
        let mut request = Request::new();
        request.fix(Package::new("vendor/b", "1.0.0"));
        request.require("vendor/a", "*");

        let generator = RuleGenerator::new(&pool);
        let rules = generator.generate(&request);

        // Should have fixed package rule
        let fixed_rules: Vec<_> = rules.rules_of_type(RuleType::Fixed).collect();
        assert!(!fixed_rules.is_empty());
    }

    #[test]
    fn test_rule_generator_stats() {
        let pool = create_test_pool();
        let mut request = Request::new();
        request.require("vendor/a", "*");

        let generator = RuleGenerator::new(&pool);
        let rules = generator.generate(&request);

        let stats = rules.stats();
        println!("Rules generated: {:?}", stats);
        assert!(stats.total > 0);
    }
}

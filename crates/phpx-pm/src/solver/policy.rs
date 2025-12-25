use std::collections::BTreeMap;
use super::pool::{Pool, PackageId};

/// Policy for selecting between candidate packages.
///
/// When multiple packages can satisfy a requirement, the policy
/// determines which one to try first.
#[derive(Debug, Clone)]
pub struct Policy {
    /// Prefer stable versions over dev
    pub prefer_stable: bool,
    /// Prefer lowest versions (for testing)
    pub prefer_lowest: bool,
}

impl Policy {
    /// Create a new policy with default settings
    pub fn new() -> Self {
        Self {
            prefer_stable: true,
            prefer_lowest: false,
        }
    }

    /// Set preference for stable versions
    pub fn prefer_stable(mut self, prefer: bool) -> Self {
        self.prefer_stable = prefer;
        self
    }

    /// Set preference for lowest versions
    pub fn prefer_lowest(mut self, prefer: bool) -> Self {
        self.prefer_lowest = prefer;
        self
    }

    /// Select the preferred package from candidates.
    ///
    /// Returns the candidates sorted by preference (best first).
    /// This implements Composer's package selection logic:
    /// 1. Prefer aliases over non-aliases (for same package name)
    /// 2. Prefer original packages over replacers
    /// 3. Prefer same vendor as the required package
    /// 4. Prefer by version (highest/lowest based on policy)
    /// 5. Fall back to package ID (pool insertion order)
    pub fn select_preferred(&self, pool: &Pool, candidates: &[PackageId]) -> Vec<PackageId> {
        self.select_preferred_for_requirement(pool, candidates, None)
    }

    /// Select preferred packages considering the required package name.
    /// This allows preferring packages from the same vendor.
    pub fn select_preferred_for_requirement(
        &self,
        pool: &Pool,
        candidates: &[PackageId],
        required_package: Option<&str>,
    ) -> Vec<PackageId> {
        if candidates.is_empty() {
            return Vec::new();
        }

        // Group candidates by package name (use BTreeMap for deterministic ordering)
        let mut by_name: BTreeMap<String, Vec<PackageId>> = BTreeMap::new();
        for &id in candidates {
            if let Some(pkg) = pool.package(id) {
                by_name.entry(pkg.name.to_lowercase()).or_default().push(id);
            }
        }

        // Sort each group by version
        for group in by_name.values_mut() {
            group.sort_by(|&a, &b| {
                self.compare_by_priority(pool, a, b, required_package, true)
            });

            // Prune to best version within each group
            *group = self.prune_to_best_version(pool, group);
        }

        // Flatten and sort across all groups
        let mut result: Vec<PackageId> = by_name.into_values().flatten().collect();

        // Final sort respecting replacers across packages
        result.sort_by(|&a, &b| {
            self.compare_by_priority(pool, a, b, required_package, false)
        });

        result
    }

    /// Compare two packages by priority (Composer's compareByPriority logic).
    fn compare_by_priority(
        &self,
        pool: &Pool,
        a: PackageId,
        b: PackageId,
        required_package: Option<&str>,
        ignore_replace: bool,
    ) -> std::cmp::Ordering {
        let pkg_a = pool.package(a);
        let pkg_b = pool.package(b);

        match (pkg_a, pkg_b) {
            (Some(pa), Some(pb)) => {
                // Prefer aliases over non-aliases for same package name
                if pa.name.to_lowercase() == pb.name.to_lowercase() {
                    let a_is_alias = pool.is_alias(a);
                    let b_is_alias = pool.is_alias(b);
                    if a_is_alias && !b_is_alias {
                        return std::cmp::Ordering::Less; // prefer a (alias)
                    }
                    if !a_is_alias && b_is_alias {
                        return std::cmp::Ordering::Greater; // prefer b (alias)
                    }
                }

                if !ignore_replace {
                    // Prefer original packages over replacers
                    // If a replaces b's name, prefer b (the original)
                    if self.replaces(pa, &pb.name) {
                        return std::cmp::Ordering::Greater; // prefer b
                    }
                    if self.replaces(pb, &pa.name) {
                        return std::cmp::Ordering::Less; // prefer a
                    }

                    // Prefer same vendor as required package
                    if let Some(req_pkg) = required_package {
                        if let Some(req_vendor) = req_pkg.split('/').next() {
                            let a_same_vendor = pa.name.starts_with(&format!("{}/", req_vendor));
                            let b_same_vendor = pb.name.starts_with(&format!("{}/", req_vendor));
                            if a_same_vendor && !b_same_vendor {
                                return std::cmp::Ordering::Less; // prefer a
                            }
                            if !a_same_vendor && b_same_vendor {
                                return std::cmp::Ordering::Greater; // prefer b
                            }
                        }
                    }
                }

                // Compare stability if prefer_stable is set
                if self.prefer_stable {
                    let stability_a = pa.stability();
                    let stability_b = pb.stability();
                    let stability_cmp = stability_a.priority().cmp(&stability_b.priority());
                    if stability_cmp != std::cmp::Ordering::Equal {
                        return stability_cmp;
                    }
                }

                // Compare versions
                let version_cmp = compare_versions(&pa.version, &pb.version);
                let version_result = if self.prefer_lowest {
                    version_cmp
                } else {
                    version_cmp.reverse()
                };

                if version_result != std::cmp::Ordering::Equal {
                    return version_result;
                }

                // Fall back to package ID (pool insertion order)
                a.cmp(&b)
            }
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    }

    /// Check if source package replaces target package name.
    fn replaces(&self, source: &crate::package::Package, target_name: &str) -> bool {
        source.replace.keys().any(|replaced| replaced.eq_ignore_ascii_case(target_name))
    }

    /// Prune list to only include the best version(s).
    /// Uses version_compare which respects stability preference.
    fn prune_to_best_version(&self, pool: &Pool, candidates: &[PackageId]) -> Vec<PackageId> {
        if candidates.is_empty() {
            return Vec::new();
        }

        let mut best = vec![candidates[0]];
        let mut best_pkg = pool.package(candidates[0]);

        for &candidate in &candidates[1..] {
            let pkg = pool.package(candidate);
            match (pkg, best_pkg) {
                (Some(p), Some(bp)) => {
                    let cmp = self.version_compare(p, bp);

                    match cmp {
                        std::cmp::Ordering::Less => {
                            // This is better
                            best = vec![candidate];
                            best_pkg = Some(p);
                        }
                        std::cmp::Ordering::Equal => {
                            // Same version, keep both
                            best.push(candidate);
                        }
                        std::cmp::Ordering::Greater => {
                            // Current best is better, skip
                        }
                    }
                }
                _ => {}
            }
        }

        best
    }

    /// Compare versions respecting stability and prefer_lowest settings.
    /// Returns Ordering::Less if a is better than b.
    fn version_compare(&self, a: &crate::package::Package, b: &crate::package::Package) -> std::cmp::Ordering {
        // First compare stability if prefer_stable is set
        if self.prefer_stable {
            let stab_a = a.stability().priority();
            let stab_b = b.stability().priority();
            if stab_a != stab_b {
                // Lower priority number = more stable = better
                return stab_a.cmp(&stab_b);
            }
        }

        // Then compare versions
        let version_cmp = compare_versions(&a.version, &b.version);
        if self.prefer_lowest {
            version_cmp
        } else {
            version_cmp.reverse()
        }
    }

    /// Select a single best package from candidates
    pub fn select_best(&self, pool: &Pool, candidates: &[PackageId]) -> Option<PackageId> {
        self.select_preferred(pool, candidates).into_iter().next()
    }
}

impl Default for Policy {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple version comparison.
/// Returns Ordering::Greater if a > b (a is newer).
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parts_a: Vec<u32> = a
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();

    let parts_b: Vec<u32> = b
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();

    let max_len = parts_a.len().max(parts_b.len());

    for i in 0..max_len {
        let pa = parts_a.get(i).copied().unwrap_or(0);
        let pb = parts_b.get(i).copied().unwrap_or(0);

        match pa.cmp(&pb) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }

    std::cmp::Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::Package;

    #[test]
    fn test_compare_versions() {
        assert_eq!(compare_versions("1.0.0", "1.0.0"), std::cmp::Ordering::Equal);
        assert_eq!(compare_versions("2.0.0", "1.0.0"), std::cmp::Ordering::Greater);
        assert_eq!(compare_versions("1.0.0", "2.0.0"), std::cmp::Ordering::Less);
        assert_eq!(compare_versions("1.10.0", "1.9.0"), std::cmp::Ordering::Greater);
        assert_eq!(compare_versions("1.0.0", "1.0.0.0"), std::cmp::Ordering::Equal);
    }

    #[test]
    fn test_policy_prefer_highest() {
        let mut pool = Pool::new();
        let _id1 = pool.add_package(Package::new("vendor/pkg", "1.0.0"));
        let id2 = pool.add_package(Package::new("vendor/pkg", "2.0.0"));
        let _id3 = pool.add_package(Package::new("vendor/pkg", "1.5.0"));

        let policy = Policy::new();
        let sorted = policy.select_preferred(&pool, &[1, 2, 3]);

        // Policy now prunes to best version, so only 2.0.0 is returned
        assert_eq!(sorted.len(), 1);
        assert_eq!(sorted[0], id2); // 2.0.0 is best (highest)
    }

    #[test]
    fn test_policy_prefer_lowest() {
        let mut pool = Pool::new();
        let id1 = pool.add_package(Package::new("vendor/pkg", "1.0.0"));
        let _id2 = pool.add_package(Package::new("vendor/pkg", "2.0.0"));
        let _id3 = pool.add_package(Package::new("vendor/pkg", "1.5.0"));

        let policy = Policy::new().prefer_lowest(true);
        let sorted = policy.select_preferred(&pool, &[1, 2, 3]);

        // Policy now prunes to best version, so only 1.0.0 is returned
        assert_eq!(sorted.len(), 1);
        assert_eq!(sorted[0], id1); // 1.0.0 is best (lowest)
    }

    #[test]
    fn test_policy_prefer_stable() {
        let mut pool = Pool::new();
        let _id1 = pool.add_package(Package::new("vendor/pkg", "2.0.0-dev"));
        let id2 = pool.add_package(Package::new("vendor/pkg", "1.0.0"));

        let policy = Policy::new().prefer_stable(true);
        let sorted = policy.select_preferred(&pool, &[1, 2]);

        // Stable (1.0.0) should be preferred even though 2.0.0 > 1.0.0
        assert_eq!(sorted.len(), 1);
        assert_eq!(sorted[0], id2);
    }

    #[test]
    fn test_policy_select_best() {
        let mut pool = Pool::new();
        pool.add_package(Package::new("vendor/pkg", "1.0.0"));
        let id2 = pool.add_package(Package::new("vendor/pkg", "2.0.0"));

        let policy = Policy::new();
        let best = policy.select_best(&pool, &[1, 2]);

        assert_eq!(best, Some(id2));
    }

    #[test]
    fn test_policy_prefer_original_over_replacer() {
        let mut pool = Pool::new();

        // Original package
        let id1 = pool.add_package(Package::new("vendor/original", "1.0.0"));

        // Replacer package
        let mut replacer = Package::new("vendor/replacer", "1.0.0");
        replacer.replace.insert("vendor/original".to_string(), "*".to_string());
        let id2 = pool.add_package(replacer);

        let policy = Policy::new();
        let sorted = policy.select_preferred_for_requirement(&pool, &[id1, id2], Some("vendor/original"));

        // Original should be preferred over replacer
        assert_eq!(sorted[0], id1);
    }
}

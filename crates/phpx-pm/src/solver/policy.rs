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
    pub fn select_preferred(&self, pool: &Pool, candidates: &[PackageId]) -> Vec<PackageId> {
        if candidates.is_empty() {
            return Vec::new();
        }

        let mut sorted: Vec<_> = candidates.to_vec();

        sorted.sort_by(|&a, &b| {
            let pkg_a = pool.package(a);
            let pkg_b = pool.package(b);

            match (pkg_a, pkg_b) {
                (Some(pa), Some(pb)) => {
                    // First, compare stability if prefer_stable is set
                    if self.prefer_stable {
                        let stability_a = pa.stability();
                        let stability_b = pb.stability();
                        let stability_cmp = stability_a.priority().cmp(&stability_b.priority());
                        if stability_cmp != std::cmp::Ordering::Equal {
                            return stability_cmp;
                        }
                    }

                    // Then compare versions
                    let version_cmp = compare_versions(&pa.version, &pb.version);
                    if self.prefer_lowest {
                        version_cmp
                    } else {
                        version_cmp.reverse()
                    }
                }
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        });

        sorted
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
        let id1 = pool.add_package(Package::new("vendor/pkg", "1.0.0"));
        let id2 = pool.add_package(Package::new("vendor/pkg", "2.0.0"));
        let id3 = pool.add_package(Package::new("vendor/pkg", "1.5.0"));

        let policy = Policy::new();
        let sorted = policy.select_preferred(&pool, &[id1, id2, id3]);

        assert_eq!(sorted, vec![id2, id3, id1]); // 2.0.0, 1.5.0, 1.0.0
    }

    #[test]
    fn test_policy_prefer_lowest() {
        let mut pool = Pool::new();
        let id1 = pool.add_package(Package::new("vendor/pkg", "1.0.0"));
        let id2 = pool.add_package(Package::new("vendor/pkg", "2.0.0"));
        let id3 = pool.add_package(Package::new("vendor/pkg", "1.5.0"));

        let policy = Policy::new().prefer_lowest(true);
        let sorted = policy.select_preferred(&pool, &[id1, id2, id3]);

        assert_eq!(sorted, vec![id1, id3, id2]); // 1.0.0, 1.5.0, 2.0.0
    }

    #[test]
    fn test_policy_prefer_stable() {
        let mut pool = Pool::new();
        let id1 = pool.add_package(Package::new("vendor/pkg", "2.0.0-dev"));
        let id2 = pool.add_package(Package::new("vendor/pkg", "1.0.0"));

        let policy = Policy::new().prefer_stable(true);
        let sorted = policy.select_preferred(&pool, &[id1, id2]);

        // Stable (1.0.0) should come before dev (2.0.0-dev) even though 2.0.0 > 1.0.0
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
}

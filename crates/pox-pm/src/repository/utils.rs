use std::collections::HashSet;
use std::sync::Arc;

use crate::json::ComposerJson;
use crate::Package;

pub struct RepositoryUtils;

impl RepositoryUtils {
    pub fn filter_required_packages(
        packages: &[Arc<Package>],
        composer_json: &ComposerJson,
    ) -> Vec<Arc<Package>> {
        Self::filter_required_packages_internal(packages, composer_json, false)
    }

    pub fn filter_required_packages_with_dev(
        packages: &[Arc<Package>],
        composer_json: &ComposerJson,
    ) -> Vec<Arc<Package>> {
        Self::filter_required_packages_internal(packages, composer_json, true)
    }

    fn filter_required_packages_internal(
        packages: &[Arc<Package>],
        composer_json: &ComposerJson,
        include_require_dev: bool,
    ) -> Vec<Arc<Package>> {
        let mut required_names: HashSet<String> = composer_json
            .require
            .keys()
            .map(|s| s.to_lowercase())
            .collect();

        if include_require_dev {
            required_names.extend(
                composer_json
                    .require_dev
                    .keys()
                    .map(|s| s.to_lowercase()),
            );
        }

        let mut name_to_packages: std::collections::HashMap<String, Vec<Arc<Package>>> =
            std::collections::HashMap::new();

        for package in packages {
            name_to_packages
                .entry(package.name.to_lowercase())
                .or_default()
                .push(Arc::clone(package));

            for provided in package.provide.keys() {
                name_to_packages
                    .entry(provided.to_lowercase())
                    .or_default()
                    .push(Arc::clone(package));
            }

            for replaced in package.replace.keys() {
                name_to_packages
                    .entry(replaced.to_lowercase())
                    .or_default()
                    .push(Arc::clone(package));
            }
        }

        let mut result: Vec<Arc<Package>> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut to_process: Vec<String> = required_names.into_iter().collect();

        while let Some(name) = to_process.pop() {
            if let Some(candidates) = name_to_packages.get(&name) {
                for package in candidates {
                    let pkg_key = package.name.to_lowercase();
                    if seen.contains(&pkg_key) {
                        continue;
                    }
                    seen.insert(pkg_key);
                    result.push(Arc::clone(package));

                    for dep_name in package.require.keys() {
                        let dep_lower = dep_name.to_lowercase();
                        if !seen.contains(&dep_lower) {
                            to_process.push(dep_lower);
                        }
                    }
                }
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_package(name: &str, requires: &[&str]) -> Arc<Package> {
        let mut pkg = Package::new(name, "1.0.0");
        for req in requires {
            pkg.require.insert(req.to_string(), "*".to_string());
        }
        Arc::new(pkg)
    }

    #[test]
    fn test_filter_required_packages_simple() {
        let packages = vec![
            make_package("vendor/a", &[]),
            make_package("vendor/b", &[]),
            make_package("vendor/c", &[]),
        ];

        let mut composer_json = ComposerJson::default();
        composer_json
            .require
            .insert("vendor/a".to_string(), "*".to_string());

        let filtered = RepositoryUtils::filter_required_packages(&packages, &composer_json);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "vendor/a");
    }

    #[test]
    fn test_filter_required_packages_transitive() {
        let packages = vec![
            make_package("vendor/a", &["vendor/b"]),
            make_package("vendor/b", &["vendor/c"]),
            make_package("vendor/c", &[]),
            make_package("vendor/d", &[]),
        ];

        let mut composer_json = ComposerJson::default();
        composer_json
            .require
            .insert("vendor/a".to_string(), "*".to_string());

        let filtered = RepositoryUtils::filter_required_packages(&packages, &composer_json);
        assert_eq!(filtered.len(), 3);

        let names: HashSet<_> = filtered.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains("vendor/a"));
        assert!(names.contains("vendor/b"));
        assert!(names.contains("vendor/c"));
        assert!(!names.contains("vendor/d"));
    }

    #[test]
    fn test_filter_required_packages_with_dev() {
        let packages = vec![
            make_package("vendor/a", &[]),
            make_package("vendor/dev", &[]),
        ];

        let mut composer_json = ComposerJson::default();
        composer_json
            .require
            .insert("vendor/a".to_string(), "*".to_string());
        composer_json
            .require_dev
            .insert("vendor/dev".to_string(), "*".to_string());

        let filtered = RepositoryUtils::filter_required_packages(&packages, &composer_json);
        assert_eq!(filtered.len(), 1);

        let filtered_with_dev =
            RepositoryUtils::filter_required_packages_with_dev(&packages, &composer_json);
        assert_eq!(filtered_with_dev.len(), 2);
    }

    #[test]
    fn test_filter_required_packages_circular() {
        let mut pkg_a = Package::new("vendor/a", "1.0.0");
        pkg_a.require.insert("vendor/b".to_string(), "*".to_string());

        let mut pkg_b = Package::new("vendor/b", "1.0.0");
        pkg_b.require.insert("vendor/a".to_string(), "*".to_string());

        let packages = vec![Arc::new(pkg_a), Arc::new(pkg_b)];

        let mut composer_json = ComposerJson::default();
        composer_json
            .require
            .insert("vendor/a".to_string(), "*".to_string());

        let filtered = RepositoryUtils::filter_required_packages(&packages, &composer_json);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_required_packages_with_provides() {
        let mut pkg_impl = Package::new("vendor/impl", "1.0.0");
        pkg_impl
            .provide
            .insert("vendor/interface".to_string(), "1.0.0".to_string());

        let packages = vec![
            Arc::new(pkg_impl),
            make_package("vendor/other", &[]),
        ];

        let mut composer_json = ComposerJson::default();
        composer_json
            .require
            .insert("vendor/interface".to_string(), "*".to_string());

        let filtered = RepositoryUtils::filter_required_packages(&packages, &composer_json);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "vendor/impl");
    }
}

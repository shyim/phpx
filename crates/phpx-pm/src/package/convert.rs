//! Conversions between Package and LockedPackage types.

use phpx_semver::VersionParser;

use super::{Autoload, AutoloadPath, Dist, Package, Source};
use crate::json::{
    LockAutoload, LockDist, LockSource, LockedPackage,
};

impl From<&LockedPackage> for Package {
    fn from(lp: &LockedPackage) -> Self {
        let normalized_version = VersionParser::new()
            .normalize(&lp.version)
            .unwrap_or_else(|_| lp.version.clone());

        let mut pkg = Package::new(&lp.name, &normalized_version);
        pkg.pretty_version = Some(lp.version.clone());
        pkg.description = lp.description.clone();
        pkg.homepage = lp.homepage.clone();
        pkg.license = lp.license.clone();
        pkg.keywords = lp.keywords.clone();
        pkg.require = lp.require.clone();
        pkg.require_dev = lp.require_dev.clone();
        pkg.conflict = lp.conflict.clone();
        pkg.provide = lp.provide.clone();
        pkg.replace = lp.replace.clone();
        pkg.suggest = lp.suggest.clone();
        pkg.bin = lp.bin.clone();
        pkg.package_type = lp.package_type.clone();
        pkg.extra = lp.extra.clone();
        pkg.notification_url = lp.notification_url.clone();
        pkg.installation_source = lp.installation_source.clone();
        pkg.default_branch = lp.default_branch;

        if let Some(ref src) = lp.source {
            pkg.source = Some(Source::new(&src.source_type, &src.url, &src.reference));
        }

        if let Some(ref dist) = lp.dist {
            let mut d = Dist::new(&dist.dist_type, &dist.url);
            if let Some(ref r) = dist.reference {
                d = d.with_reference(r);
            }
            if let Some(ref s) = dist.shasum {
                d = d.with_shasum(s);
            }
            pkg.dist = Some(d);
        }

        if !lp.autoload.is_empty() {
            pkg.autoload = Some(Autoload::from(&lp.autoload));
        }

        if !lp.autoload_dev.is_empty() {
            pkg.autoload_dev = Some(Autoload::from(&lp.autoload_dev));
        }

        if let Some(ref time_str) = lp.time {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(time_str) {
                pkg.time = Some(dt.with_timezone(&chrono::Utc));
            }
        }

        pkg
    }
}

impl From<LockedPackage> for Package {
    fn from(lp: LockedPackage) -> Self {
        Package::from(&lp)
    }
}

impl From<&Package> for LockedPackage {
    fn from(pkg: &Package) -> Self {
        LockedPackage {
            name: pkg.name.clone(),
            version: pkg.pretty_version().to_string(),
            source: pkg.source.as_ref().map(LockSource::from),
            dist: pkg.dist.as_ref().map(LockDist::from),
            require: pkg.require.clone(),
            require_dev: pkg.require_dev.clone(),
            conflict: pkg.conflict.clone(),
            provide: pkg.provide.clone(),
            replace: pkg.replace.clone(),
            suggest: pkg.suggest.clone(),
            bin: pkg.bin.clone(),
            package_type: pkg.package_type.clone(),
            extra: pkg.extra.clone(),
            autoload: pkg.autoload.as_ref().map(LockAutoload::from).unwrap_or_default(),
            autoload_dev: pkg.autoload_dev.as_ref().map(LockAutoload::from).unwrap_or_default(),
            notification_url: pkg.notification_url.clone(),
            description: pkg.description.clone(),
            homepage: pkg.homepage.clone(),
            keywords: pkg.keywords.clone(),
            license: pkg.license.clone(),
            time: pkg.time.map(|t| t.to_rfc3339()),
            installation_source: pkg.installation_source.clone(),
            default_branch: pkg.default_branch,
            ..Default::default()
        }
    }
}

impl From<Package> for LockedPackage {
    fn from(pkg: Package) -> Self {
        LockedPackage::from(&pkg)
    }
}

impl From<&Source> for LockSource {
    fn from(s: &Source) -> Self {
        LockSource {
            source_type: s.source_type.clone(),
            url: s.url.clone(),
            reference: s.reference.clone(),
        }
    }
}

impl From<&Dist> for LockDist {
    fn from(d: &Dist) -> Self {
        LockDist {
            dist_type: d.dist_type.clone(),
            url: d.url.clone(),
            reference: d.reference.clone(),
            shasum: d.shasum.clone(),
        }
    }
}

impl From<&LockAutoload> for Autoload {
    fn from(lock_autoload: &LockAutoload) -> Self {
        let mut autoload = Autoload::default();

        for (ns, v) in &lock_autoload.psr4 {
            autoload.psr4.insert(ns.clone(), json_value_to_paths(v));
        }

        for (ns, v) in &lock_autoload.psr0 {
            autoload.psr0.insert(ns.clone(), json_value_to_paths(v));
        }

        autoload.classmap = lock_autoload.classmap.clone();
        autoload.files = lock_autoload.files.clone();
        autoload.exclude_from_classmap = lock_autoload.exclude_from_classmap.clone();

        autoload
    }
}

impl From<&Autoload> for LockAutoload {
    fn from(a: &Autoload) -> Self {
        LockAutoload {
            psr4: a.psr4.iter()
                .map(|(k, v)| (k.clone(), serde_json::to_value(v).unwrap_or(serde_json::Value::Null)))
                .collect(),
            psr0: a.psr0.iter()
                .map(|(k, v)| (k.clone(), serde_json::to_value(v).unwrap_or(serde_json::Value::Null)))
                .collect(),
            classmap: a.classmap.clone(),
            files: a.files.clone(),
            exclude_from_classmap: a.exclude_from_classmap.clone(),
        }
    }
}

fn json_value_to_paths(value: &serde_json::Value) -> AutoloadPath {
    match value {
        serde_json::Value::String(s) => AutoloadPath::Single(s.clone()),
        serde_json::Value::Array(arr) => {
            let paths: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            AutoloadPath::Multiple(paths)
        }
        _ => AutoloadPath::Single(String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_locked_package_to_package() {
        let locked = LockedPackage {
            name: "vendor/package".to_string(),
            version: "1.0.0".to_string(),
            package_type: "library".to_string(),
            description: Some("A test package".to_string()),
            ..Default::default()
        };

        let pkg = Package::from(&locked);
        assert_eq!(pkg.name, "vendor/package");
        assert_eq!(pkg.version, "1.0.0.0");
        assert_eq!(pkg.pretty_version, Some("1.0.0".to_string()));
        assert_eq!(pkg.description, Some("A test package".to_string()));
    }

    #[test]
    fn test_package_to_locked_package() {
        let mut pkg = Package::new("vendor/package", "1.0.0");
        pkg.description = Some("A test package".to_string());

        let locked = LockedPackage::from(&pkg);
        assert_eq!(locked.name, "vendor/package");
        assert_eq!(locked.version, "1.0.0");
        assert_eq!(locked.description, Some("A test package".to_string()));
    }

    #[test]
    fn test_roundtrip_conversion() {
        let mut original = Package::new("vendor/package", "1.2.3");
        original.description = Some("Test description".to_string());
        original.homepage = Some("https://example.com".to_string());
        original.license = vec!["MIT".to_string()];
        original.require.insert("php".to_string(), ">=8.0".to_string());
        original.require.insert("other/pkg".to_string(), "^1.0".to_string());

        let locked = LockedPackage::from(&original);
        let converted = Package::from(&locked);

        assert_eq!(converted.name, original.name);
        assert_eq!(converted.description, original.description);
        assert_eq!(converted.homepage, original.homepage);
        assert_eq!(converted.license, original.license);
        assert_eq!(converted.require, original.require);
    }
}

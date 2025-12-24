//! Classmap generator - scans PHP files for class definitions.

use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::Result;

/// Generates a classmap by scanning PHP files
pub struct ClassMapGenerator {
    /// Regex for matching class/interface/trait/enum definitions
    class_regex: Regex,
    /// Regex for matching namespace declarations
    namespace_regex: Regex,
}

impl ClassMapGenerator {
    /// Create a new classmap generator
    pub fn new() -> Self {
        Self {
            // Match class, interface, trait, or enum definitions
            class_regex: Regex::new(
                r"(?m)^\s*(?:abstract\s+|final\s+)?(?:class|interface|trait|enum)\s+([a-zA-Z_\x80-\xff][a-zA-Z0-9_\x80-\xff]*)"
            ).unwrap(),
            // Match namespace declarations
            namespace_regex: Regex::new(
                r"(?m)^\s*namespace\s+([a-zA-Z_\x80-\xff][a-zA-Z0-9_\x80-\xff\\]*)\s*[;{]"
            ).unwrap(),
        }
    }

    /// Generate classmap for a directory
    pub fn generate(&self, path: &Path) -> Result<HashMap<String, PathBuf>> {
        let mut classmap = HashMap::new();

        if !path.exists() {
            return Ok(classmap);
        }

        for entry in WalkDir::new(path)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let file_path = entry.path();

            // Only process PHP files
            if !Self::is_php_file(file_path) {
                continue;
            }

            // Read and parse the file
            if let Ok(content) = std::fs::read_to_string(file_path) {
                let classes = self.extract_classes(&content);
                for class in classes {
                    classmap.insert(class, file_path.to_path_buf());
                }
            }
        }

        Ok(classmap)
    }

    /// Generate classmap for multiple directories
    pub fn generate_from_paths(&self, paths: &[PathBuf]) -> Result<HashMap<String, PathBuf>> {
        let mut classmap = HashMap::new();

        for path in paths {
            let map = self.generate(path)?;
            classmap.extend(map);
        }

        Ok(classmap)
    }

    /// Extract class names from PHP content
    fn extract_classes(&self, content: &str) -> Vec<String> {
        let mut classes = Vec::new();

        // Find namespace
        let namespace = self.namespace_regex
            .captures(content)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string());

        // Find all class definitions
        for cap in self.class_regex.captures_iter(content) {
            if let Some(class_name) = cap.get(1) {
                let full_name = match &namespace {
                    Some(ns) => format!("{}\\{}", ns, class_name.as_str()),
                    None => class_name.as_str().to_string(),
                };
                classes.push(full_name);
            }
        }

        classes
    }

    /// Check if a file is a PHP file
    fn is_php_file(path: &Path) -> bool {
        path.extension()
            .map(|ext| ext.eq_ignore_ascii_case("php"))
            .unwrap_or(false)
    }
}

impl Default for ClassMapGenerator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;

    #[test]
    fn test_extract_class() {
        let gen = ClassMapGenerator::new();
        let content = r#"<?php
class MyClass {
}
"#;
        let classes = gen.extract_classes(content);
        assert_eq!(classes, vec!["MyClass"]);
    }

    #[test]
    fn test_extract_namespaced_class() {
        let gen = ClassMapGenerator::new();
        let content = r#"<?php
namespace Vendor\Package;

class MyClass {
}
"#;
        let classes = gen.extract_classes(content);
        assert_eq!(classes, vec!["Vendor\\Package\\MyClass"]);
    }

    #[test]
    fn test_extract_interface() {
        let gen = ClassMapGenerator::new();
        let content = r#"<?php
namespace App;

interface MyInterface {
}
"#;
        let classes = gen.extract_classes(content);
        assert_eq!(classes, vec!["App\\MyInterface"]);
    }

    #[test]
    fn test_extract_trait() {
        let gen = ClassMapGenerator::new();
        let content = r#"<?php
namespace App\Traits;

trait MyTrait {
}
"#;
        let classes = gen.extract_classes(content);
        assert_eq!(classes, vec!["App\\Traits\\MyTrait"]);
    }

    #[test]
    fn test_extract_enum() {
        let gen = ClassMapGenerator::new();
        let content = r#"<?php
namespace App\Enums;

enum Status {
    case Active;
    case Inactive;
}
"#;
        let classes = gen.extract_classes(content);
        assert_eq!(classes, vec!["App\\Enums\\Status"]);
    }

    #[test]
    fn test_extract_abstract_class() {
        let gen = ClassMapGenerator::new();
        let content = r#"<?php
namespace App;

abstract class AbstractBase {
}
"#;
        let classes = gen.extract_classes(content);
        assert_eq!(classes, vec!["App\\AbstractBase"]);
    }

    #[test]
    fn test_extract_final_class() {
        let gen = ClassMapGenerator::new();
        let content = r#"<?php
namespace App;

final class FinalClass {
}
"#;
        let classes = gen.extract_classes(content);
        assert_eq!(classes, vec!["App\\FinalClass"]);
    }

    #[test]
    fn test_generate_from_directory() {
        let temp_dir = TempDir::new().unwrap();

        // Create test PHP file
        let php_content = r#"<?php
namespace Test;

class TestClass {
}
"#;
        let file_path = temp_dir.path().join("TestClass.php");
        fs::write(&file_path, php_content).unwrap();

        let gen = ClassMapGenerator::new();
        let classmap = gen.generate(temp_dir.path()).unwrap();

        assert_eq!(classmap.len(), 1);
        assert!(classmap.contains_key("Test\\TestClass"));
    }

    #[test]
    fn test_is_php_file() {
        assert!(ClassMapGenerator::is_php_file(Path::new("test.php")));
        assert!(ClassMapGenerator::is_php_file(Path::new("test.PHP")));
        assert!(!ClassMapGenerator::is_php_file(Path::new("test.txt")));
        assert!(!ClassMapGenerator::is_php_file(Path::new("test")));
    }
}

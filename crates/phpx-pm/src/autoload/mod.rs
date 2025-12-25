//! Autoload generation for PHP packages.
//!
//! This module generates the vendor/autoload.php and related files
//! that enable automatic class loading in PHP.

mod generator;
mod classmap;

pub use generator::{AutoloadGenerator, AutoloadConfig, PackageAutoload, RootPackageInfo};
pub use classmap::ClassMapGenerator;

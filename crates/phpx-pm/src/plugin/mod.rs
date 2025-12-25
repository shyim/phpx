//! Plugin system for ported Composer plugins.
//!
//! This module provides a registry of Composer plugins that have been
//! ported to native Rust implementations for phpx. Since phpx cannot
//! execute PHP-based Composer plugins, popular plugins are manually
//! ported and registered here.

mod composer_bin;
mod phpstan_extension_installer;
mod registry;
mod symfony_runtime;

pub use composer_bin::BinConfig;
pub use registry::PluginRegistry;

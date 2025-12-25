//! Plugin system for ported Composer plugins.
//!
//! This module provides a registry of Composer plugins that have been
//! ported to native Rust implementations for phpx. Since phpx cannot
//! execute PHP-based Composer plugins, popular plugins are manually
//! ported and registered here.

mod registry;
mod symfony_runtime;

pub use registry::PluginRegistry;

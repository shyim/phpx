// Package model for Composer packages
//
// This module provides structs and types for representing Composer packages,
// including dependencies, autoload configuration, source/dist information, etc.

mod autoload;
mod link;
mod package;
mod source;

pub use autoload::{Autoload, AutoloadPath};
pub use link::{Link, LinkType};
pub use package::{
    Abandoned, ArchiveConfig, Author, Funding, Package, ScriptHandler, Scripts, Stability,
    Support,
};
pub use source::{Dist, Mirror, Source};

//! Package installation system.
//!
//! This module handles the installation, update, and removal of packages
//! into the vendor directory.

mod manager;
mod library;
mod binary;

pub use manager::{InstallationManager, InstallConfig};
pub use library::LibraryInstaller;
pub use binary::BinaryInstaller;

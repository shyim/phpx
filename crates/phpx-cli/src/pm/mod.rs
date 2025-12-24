//! Package manager subcommands.

mod audit;
mod bump;
mod exec;
mod search;
mod show;
mod validate;
mod dump_autoload;
mod why;
mod outdated;
mod clear_cache;
pub mod run;
pub mod platform;
pub mod scripts;

use clap::Subcommand;
use anyhow::Result;

pub use audit::AuditArgs;
pub use bump::BumpArgs;
pub use exec::ExecArgs;
pub use search::SearchArgs;
pub use show::ShowArgs;
pub use validate::ValidateArgs;
pub use dump_autoload::DumpAutoloadArgs;
pub use why::WhyArgs;
pub use outdated::OutdatedArgs;
pub use clear_cache::ClearCacheArgs;
pub use run::RunArgs;

/// Package manager subcommands
#[derive(Subcommand, Debug)]
pub enum PmCommands {
    /// Check for security vulnerabilities in installed packages
    Audit(AuditArgs),

    /// Bump version constraints in composer.json to locked versions
    Bump(BumpArgs),

    /// Execute a vendored binary/script
    Exec(ExecArgs),

    /// Search for packages on Packagist
    Search(SearchArgs),

    /// Show information about packages
    Show(ShowArgs),

    /// Validate composer.json and composer.lock
    Validate(ValidateArgs),

    /// Regenerate the autoloader
    #[command(name = "dump-autoload", alias = "dumpautoload")]
    DumpAutoload(DumpAutoloadArgs),

    /// Show why a package is installed
    Why(WhyArgs),

    /// Show outdated packages
    Outdated(OutdatedArgs),

    /// Clear the Composer cache
    #[command(name = "clear-cache", alias = "clearcache")]
    ClearCache(ClearCacheArgs),
}

/// Execute a package manager command
pub async fn execute(command: PmCommands) -> Result<i32> {
    match command {
        PmCommands::Audit(args) => audit::execute(args).await,
        PmCommands::Bump(args) => bump::execute(args).await,
        PmCommands::Exec(args) => exec::execute(args).await,
        PmCommands::Search(args) => search::execute(args).await,
        PmCommands::Show(args) => show::execute(args).await,
        PmCommands::Validate(args) => validate::execute(args).await,
        PmCommands::DumpAutoload(args) => dump_autoload::execute(args).await,
        PmCommands::Why(args) => why::execute(args).await,
        PmCommands::Outdated(args) => outdated::execute(args).await,
        PmCommands::ClearCache(args) => clear_cache::execute(args).await,
    }
}

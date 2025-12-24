//! SAT-based dependency resolver for Composer packages.
//!
//! This module implements a CDCL (Conflict-Driven Clause Learning) SAT solver
//! specifically designed for package dependency resolution. The implementation
//! follows Composer's solver design.
//!
//! # Architecture
//!
//! The solver consists of several key components:
//!
//! - [`Pool`]: Registry of all available packages with lookup by name/constraint
//! - [`Request`]: Specification of what needs to be resolved
//! - [`RuleSet`]: Collection of SAT clauses representing dependencies
//! - [`Solver`]: The main CDCL algorithm implementation
//!
//! # Algorithm Overview
//!
//! 1. **Rule Generation**: Convert dependency graph to SAT clauses
//! 2. **Unit Propagation**: Force decisions from unit clauses
//! 3. **Decision Making**: Choose package versions using policy
//! 4. **Conflict Analysis**: Learn from conflicts to avoid repeating mistakes
//! 5. **Backtracking**: Revert to appropriate level on conflict
//!
//! # Example
//!
//! ```ignore
//! use phpx_pm::solver::{Pool, Request, Solver, Policy};
//!
//! let pool = Pool::new();
//! // ... add packages to pool
//!
//! let request = Request::new();
//! // ... add requirements to request
//!
//! let policy = Policy::default();
//! let solver = Solver::new(&pool, &policy);
//!
//! match solver.solve(&request) {
//!     Ok(transaction) => println!("Solution found!"),
//!     Err(problems) => println!("No solution: {:?}", problems),
//! }
//! ```

mod pool;
mod request;
mod rule;
mod rule_set;
mod decisions;
mod watch_graph;
mod rule_generator;
mod solver;
mod problem;
mod transaction;
mod policy;

#[cfg(test)]
mod tests;

pub use pool::{Pool, PoolBuilder};
pub use request::Request;
pub use rule::{Rule, RuleType, Literal};
pub use rule_set::RuleSet;
pub use decisions::Decisions;
pub use solver::Solver;
pub use problem::Problem;
pub use transaction::{Transaction, Operation};
pub use policy::Policy;

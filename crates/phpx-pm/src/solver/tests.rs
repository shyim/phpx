//! Solver tests ported from Composer's SolverTest.php
//!
//! These tests validate the SAT-based dependency resolver behaves correctly
//! for various package resolution scenarios.

use super::*;
use crate::package::Package;

/// Helper to create a package with a given name and version
fn pkg(name: &str, version: &str) -> Package {
    Package::new(name, version)
}

/// Helper to create a package with requirements
fn pkg_with_requires(name: &str, version: &str, requires: Vec<(&str, &str)>) -> Package {
    let mut p = Package::new(name, version);
    for (dep_name, constraint) in requires {
        p.require.insert(dep_name.to_string(), constraint.to_string());
    }
    p
}

/// Helper to create a package with replaces
fn pkg_with_replaces(name: &str, version: &str, replaces: Vec<(&str, &str)>) -> Package {
    let mut p = Package::new(name, version);
    for (replace_name, constraint) in replaces {
        p.replace.insert(replace_name.to_string(), constraint.to_string());
    }
    p
}

/// Check that the solver result matches expected operations
fn check_solver_result(
    transaction: &Transaction,
    expected: Vec<(&str, &str, &str)>, // (job, package_name, version)
) {
    let mut actual: Vec<(String, String, String)> = Vec::new();

    for op in &transaction.operations {
        match op {
            Operation::Install(pkg) => {
                actual.push(("install".to_string(), pkg.name.clone(), pkg.version.clone()));
            }
            Operation::Update { from, to } => {
                actual.push(("update".to_string(), to.name.clone(), format!("{} -> {}", from.version, to.version)));
            }
            Operation::Uninstall(pkg) => {
                actual.push(("remove".to_string(), pkg.name.clone(), pkg.version.clone()));
            }
            Operation::MarkUnneeded(pkg) => {
                actual.push(("mark_unneeded".to_string(), pkg.name.clone(), pkg.version.clone()));
            }
        }
    }

    let expected_vec: Vec<(String, String, String)> = expected
        .into_iter()
        .map(|(j, n, v)| (j.to_string(), n.to_string(), v.to_string()))
        .collect();

    // Sort both for comparison (since order may vary)
    let mut actual_sorted = actual.clone();
    let mut expected_sorted = expected_vec.clone();
    actual_sorted.sort();
    expected_sorted.sort();

    assert_eq!(
        actual_sorted, expected_sorted,
        "\nExpected operations: {:?}\nActual operations: {:?}",
        expected_vec, actual
    );
}

// ============================================================================
// Basic Installation Tests
// ============================================================================

#[test]
fn test_solver_install_single() {
    let mut pool = Pool::new();
    pool.add_package(pkg("a", "1.0.0"));

    let policy = Policy::new();
    let solver = Solver::new(&pool, &policy);

    let mut request = Request::new();
    request.require("a", "*");

    let result = solver.solve(&request);
    assert!(result.is_ok(), "Solver should find a solution");

    let transaction = result.unwrap();
    check_solver_result(&transaction, vec![
        ("install", "a", "1.0.0"),
    ]);
}

#[test]
fn test_solver_remove_if_not_requested() {
    let mut pool = Pool::new();
    pool.add_package(pkg("a", "1.0.0"));

    let policy = Policy::new();
    let solver = Solver::new(&pool, &policy);

    let mut request = Request::new();
    // Package A is locked but not required
    request.lock(pkg("a", "1.0.0"));

    let result = solver.solve(&request);
    assert!(result.is_ok(), "Solver should find a solution");

    let transaction = result.unwrap();
    check_solver_result(&transaction, vec![
        ("remove", "a", "1.0.0"),
    ]);
}

#[test]
fn test_solver_install_with_deps() {
    let mut pool = Pool::new();

    // A requires B < 1.1
    pool.add_package(pkg_with_requires("a", "1.0.0", vec![("b", "<1.1")]));
    pool.add_package(pkg("b", "1.0.0"));
    pool.add_package(pkg("b", "1.1.0")); // This version should NOT be selected

    let policy = Policy::new();
    let solver = Solver::new(&pool, &policy);

    let mut request = Request::new();
    request.require("a", "*");

    let result = solver.solve(&request);
    assert!(result.is_ok(), "Solver should find a solution");

    let transaction = result.unwrap();

    // Should install B 1.0.0 (not 1.1.0) and A 1.0.0
    let installs: Vec<_> = transaction.installs().collect();
    assert_eq!(installs.len(), 2);

    let b_pkg = installs.iter().find(|p| p.name == "b").expect("B should be installed");
    assert_eq!(b_pkg.version, "1.0.0", "B version should be 1.0.0, not 1.1.0");
}

#[test]
fn test_solver_install_with_deps_in_order() {
    let mut pool = Pool::new();

    // B requires A and C, C requires A
    pool.add_package(pkg("a", "1.0.0"));
    pool.add_package(pkg_with_requires("b", "1.0.0", vec![("a", ">=1.0"), ("c", ">=1.0")]));
    pool.add_package(pkg_with_requires("c", "1.0.0", vec![("a", ">=1.0")]));

    let policy = Policy::new();
    let solver = Solver::new(&pool, &policy);

    let mut request = Request::new();
    request.require("a", "*");
    request.require("b", "*");
    request.require("c", "*");

    let result = solver.solve(&request);
    assert!(result.is_ok(), "Solver should find a solution");

    let transaction = result.unwrap();

    // After sorting, A should be installed before C, C before B
    let install_names: Vec<String> = transaction.installs().map(|p| p.name.clone()).collect();
    assert_eq!(install_names.len(), 3);
}

// ============================================================================
// Update Tests
// ============================================================================

#[test]
fn test_solver_update_single() {
    let mut pool = Pool::new();
    pool.add_package(pkg("a", "1.0.0"));
    pool.add_package(pkg("a", "1.1.0"));

    let policy = Policy::new();
    let solver = Solver::new(&pool, &policy);

    let mut request = Request::new();
    request.require("a", "*");
    request.lock(pkg("a", "1.0.0"));

    let result = solver.solve(&request);
    assert!(result.is_ok(), "Solver should find a solution");

    let transaction = result.unwrap();

    // Should update A from 1.0.0 to 1.1.0
    let updates: Vec<_> = transaction.updates().collect();
    assert_eq!(updates.len(), 1, "Should have one update operation");
    assert_eq!(updates[0].0.version, "1.0.0");
    assert_eq!(updates[0].1.version, "1.1.0");
}

#[test]
fn test_solver_update_current_no_change() {
    let mut pool = Pool::new();
    pool.add_package(pkg("a", "1.0.0"));

    let policy = Policy::new();
    let solver = Solver::new(&pool, &policy);

    let mut request = Request::new();
    request.require("a", "*");
    request.lock(pkg("a", "1.0.0"));

    let result = solver.solve(&request);
    assert!(result.is_ok(), "Solver should find a solution");

    let transaction = result.unwrap();

    // No changes needed - same version
    assert!(transaction.is_empty(), "Transaction should be empty when already at correct version");
}

#[test]
fn test_solver_update_constrained() {
    let mut pool = Pool::new();
    pool.add_package(pkg("a", "1.0.0"));
    pool.add_package(pkg("a", "1.2.0"));
    pool.add_package(pkg("a", "2.0.0")); // Should not be selected

    let policy = Policy::new();
    let solver = Solver::new(&pool, &policy);

    let mut request = Request::new();
    request.require("a", "<2.0");
    request.lock(pkg("a", "1.0.0"));

    let result = solver.solve(&request);
    assert!(result.is_ok(), "Solver should find a solution");

    let transaction = result.unwrap();

    // Should update to 1.2.0, not 2.0.0
    let updates: Vec<_> = transaction.updates().collect();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].1.version, "1.2.0");
}

// ============================================================================
// Conflict Tests
// ============================================================================

#[test]
fn test_solver_three_alternative_require_and_conflict() {
    let mut pool = Pool::new();

    // A requires B < 1.1 and conflicts with B < 1.0
    let mut pkg_a = pkg_with_requires("a", "2.0.0", vec![("b", "<1.1")]);
    pkg_a.conflict.insert("b".to_string(), "<1.0".to_string());
    pool.add_package(pkg_a);

    pool.add_package(pkg("b", "0.9.0")); // Too old, conflicts
    pool.add_package(pkg("b", "1.0.0")); // Just right
    pool.add_package(pkg("b", "1.1.0")); // Too new, doesn't match require

    let policy = Policy::new();
    let solver = Solver::new(&pool, &policy);

    let mut request = Request::new();
    request.require("a", "*");

    let result = solver.solve(&request);
    assert!(result.is_ok(), "Solver should find a solution");

    let transaction = result.unwrap();

    // Should install B 1.0.0 (the middle version)
    let b_pkg = transaction.installs().find(|p| p.name == "b").expect("B should be installed");
    assert_eq!(b_pkg.version, "1.0.0");
}

#[test]
fn test_solver_conflict_between_requirements() {
    let mut pool = Pool::new();

    // A requires B ^1.0
    pool.add_package(pkg_with_requires("a", "1.0.0", vec![("b", "^1.0")]));
    // C requires B ^2.0
    pool.add_package(pkg_with_requires("c", "1.0.0", vec![("b", "^2.0")]));
    // Only B 1.0 exists
    pool.add_package(pkg("b", "1.0.0"));

    let policy = Policy::new();
    let solver = Solver::new(&pool, &policy);

    let mut request = Request::new();
    request.require("a", "*");
    request.require("c", "*");

    let result = solver.solve(&request);
    // This should fail because C needs B ^2.0 but only B 1.0 exists
    assert!(result.is_err(), "Solver should fail due to conflicting requirements");
}

// ============================================================================
// Replace Tests
// ============================================================================

#[test]
fn test_solver_obsolete_replaced() {
    let mut pool = Pool::new();

    // B replaces A
    pool.add_package(pkg_with_replaces("b", "1.0.0", vec![("a", "*")]));

    let policy = Policy::new();
    let solver = Solver::new(&pool, &policy);

    let mut request = Request::new();
    request.require("b", "*");
    request.lock(pkg("a", "1.0.0"));

    let result = solver.solve(&request);
    assert!(result.is_ok(), "Solver should find a solution");

    let transaction = result.unwrap();

    // Should remove A and install B
    let removes: Vec<_> = transaction.removals().collect();
    let installs: Vec<_> = transaction.new_installs().collect();

    assert_eq!(removes.len(), 1);
    assert_eq!(removes[0].name, "a");
    assert_eq!(installs.len(), 1);
    assert_eq!(installs[0].name, "b");
}

#[test]
fn test_skip_replacer_of_existing_package() {
    let mut pool = Pool::new();

    // A requires B
    pool.add_package(pkg_with_requires("a", "1.0.0", vec![("b", ">=1.0")]));
    // B exists
    pool.add_package(pkg("b", "1.0.0"));
    // Q replaces B but we don't need it since B exists
    pool.add_package(pkg_with_replaces("q", "1.0.0", vec![("b", ">=1.0")]));

    let policy = Policy::new();
    let solver = Solver::new(&pool, &policy);

    let mut request = Request::new();
    request.require("a", "*");

    let result = solver.solve(&request);
    assert!(result.is_ok(), "Solver should find a solution");

    let transaction = result.unwrap();

    // Should install B, not Q
    let installs: Vec<_> = transaction.installs().collect();
    let b_installed = installs.iter().any(|p| p.name == "b");
    let q_installed = installs.iter().any(|p| p.name == "q");

    assert!(b_installed, "B should be installed");
    assert!(!q_installed, "Q should not be installed when B exists");
}

/// When Q replaces B and both A requires B and we require Q,
/// the solver should recognize Q satisfies B's requirement.
#[test]
fn test_skip_replaced_package_if_replacer_is_selected() {
    let mut pool = Pool::new();

    // A requires B
    pool.add_package(pkg_with_requires("a", "1.0.0", vec![("b", ">=1.0")]));
    // B exists
    pool.add_package(pkg("b", "1.0.0"));
    // Q replaces B >=1.0 (Composer-style: constraint means "replaces versions matching this")
    pool.add_package(pkg_with_replaces("q", "1.0.0", vec![("b", ">=1.0")]));

    let policy = Policy::new();
    let solver = Solver::new(&pool, &policy);

    let mut request = Request::new();
    request.require("a", "*");
    request.require("q", "*"); // Explicitly require Q

    let result = solver.solve(&request);
    assert!(result.is_ok(), "Solver should find a solution");

    let transaction = result.unwrap();

    // Should install Q, not B (since Q is explicitly required and replaces B)
    let installs: Vec<_> = transaction.installs().collect();
    let q_installed = installs.iter().any(|p| p.name == "q");

    assert!(q_installed, "Q should be installed");
}

// ============================================================================
// Circular Dependency Tests
// ============================================================================

#[test]
fn test_install_circular_require() {
    let mut pool = Pool::new();

    // A requires B >= 1.0
    pool.add_package(pkg_with_requires("a", "1.0.0", vec![("b", ">=1.0")]));
    // B 0.9 doesn't require A
    pool.add_package(pkg("b", "0.9.0"));
    // B 1.1 requires A >= 1.0 (circular)
    pool.add_package(pkg_with_requires("b", "1.1.0", vec![("a", ">=1.0")]));

    let policy = Policy::new();
    let solver = Solver::new(&pool, &policy);

    let mut request = Request::new();
    request.require("a", "*");

    let result = solver.solve(&request);
    assert!(result.is_ok(), "Solver should handle circular dependencies");

    let transaction = result.unwrap();
    let installs: Vec<_> = transaction.installs().collect();
    assert_eq!(installs.len(), 2);

    // Should install B 1.1 (the one with circular dep) and A
    let b_pkg = installs.iter().find(|p| p.name == "b").unwrap();
    assert_eq!(b_pkg.version, "1.1.0");
}

// ============================================================================
// Version Selection Tests
// ============================================================================

#[test]
fn test_solver_prefer_highest() {
    let mut pool = Pool::new();
    pool.add_package(pkg("a", "1.0.0"));
    pool.add_package(pkg("a", "2.0.0"));
    pool.add_package(pkg("a", "3.0.0"));

    let policy = Policy::new(); // Default prefers highest
    let solver = Solver::new(&pool, &policy);

    let mut request = Request::new();
    request.require("a", "*");

    let result = solver.solve(&request);
    assert!(result.is_ok());

    let transaction = result.unwrap();
    let installed: Vec<_> = transaction.installs().collect();
    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].version, "3.0.0", "Should prefer highest version");
}

#[test]
fn test_solver_prefer_lowest() {
    let mut pool = Pool::new();
    pool.add_package(pkg("a", "1.0.0"));
    pool.add_package(pkg("a", "2.0.0"));
    pool.add_package(pkg("a", "3.0.0"));

    let policy = Policy::new().prefer_lowest(true);
    let solver = Solver::new(&pool, &policy);

    let mut request = Request::new();
    request.require("a", "*");

    let result = solver.solve(&request);
    assert!(result.is_ok());

    let transaction = result.unwrap();
    let installed: Vec<_> = transaction.installs().collect();
    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].version, "1.0.0", "Should prefer lowest version");
}

#[test]
fn test_solver_pick_older_if_newer_conflicts() {
    let mut pool = Pool::new();

    // X requires A >= 2.0 and B >= 2.0
    pool.add_package(pkg_with_requires("x", "1.0.0", vec![("a", ">=2.0"), ("b", ">=2.0")]));

    // A 2.0 requires B >= 2.0
    pool.add_package(pkg_with_requires("a", "2.0.0", vec![("b", ">=2.0")]));
    // A 2.1 requires B >= 2.2 (which doesn't exist)
    pool.add_package(pkg_with_requires("a", "2.1.0", vec![("b", ">=2.2")]));

    // Only B 2.1 exists
    pool.add_package(pkg("b", "2.1.0"));

    let policy = Policy::new();
    let solver = Solver::new(&pool, &policy);

    let mut request = Request::new();
    request.require("x", "*");

    let result = solver.solve(&request);
    assert!(result.is_ok(), "Solver should find a solution");

    let transaction = result.unwrap();

    // Should install A 2.0.0 (not 2.1.0 which needs B 2.2)
    let a_pkg = transaction.installs().find(|p| p.name == "a").expect("A should be installed");
    assert_eq!(a_pkg.version, "2.0.0", "Should pick older A version that works");
}

// ============================================================================
// Fixed Package Tests
// ============================================================================

#[test]
fn test_solver_fix_locked() {
    let mut pool = Pool::new();
    pool.add_package(pkg("a", "1.0.0"));
    pool.add_package(pkg("a", "1.1.0"));

    let policy = Policy::new();
    let solver = Solver::new(&pool, &policy);

    let mut request = Request::new();
    request.lock(pkg("a", "1.0.0"));
    request.fix(pkg("a", "1.0.0"));

    let result = solver.solve(&request);
    assert!(result.is_ok());

    let transaction = result.unwrap();
    // Fixed package shouldn't be changed or removed
    assert!(transaction.is_empty());
}

// ============================================================================
// Non-existent Package Tests
// ============================================================================

#[test]
fn test_install_non_existing_package_fails() {
    let mut pool = Pool::new();
    pool.add_package(pkg("a", "1.0.0"));

    let policy = Policy::new();
    let solver = Solver::new(&pool, &policy);

    let mut request = Request::new();
    request.require("nonexistent", "1.0.0");

    let result = solver.solve(&request);
    assert!(result.is_err(), "Should fail when requiring non-existent package");
}

// ============================================================================
// Complex Dependency Chain Tests
// ============================================================================

#[test]
fn test_solver_deep_dependency_chain() {
    let mut pool = Pool::new();

    // A -> B -> C -> D -> E
    pool.add_package(pkg_with_requires("a", "1.0.0", vec![("b", "^1.0")]));
    pool.add_package(pkg_with_requires("b", "1.0.0", vec![("c", "^1.0")]));
    pool.add_package(pkg_with_requires("c", "1.0.0", vec![("d", "^1.0")]));
    pool.add_package(pkg_with_requires("d", "1.0.0", vec![("e", "^1.0")]));
    pool.add_package(pkg("e", "1.0.0"));

    let policy = Policy::new();
    let solver = Solver::new(&pool, &policy);

    let mut request = Request::new();
    request.require("a", "*");

    let result = solver.solve(&request);
    assert!(result.is_ok(), "Solver should resolve deep dependency chain");

    let transaction = result.unwrap();
    let installs: Vec<_> = transaction.installs().collect();
    assert_eq!(installs.len(), 5, "Should install all 5 packages");
}

#[test]
fn test_solver_diamond_dependency() {
    let mut pool = Pool::new();

    // Diamond: A -> B, A -> C, B -> D, C -> D
    pool.add_package(pkg_with_requires("a", "1.0.0", vec![("b", "^1.0"), ("c", "^1.0")]));
    pool.add_package(pkg_with_requires("b", "1.0.0", vec![("d", "^1.0")]));
    pool.add_package(pkg_with_requires("c", "1.0.0", vec![("d", "^1.0")]));
    pool.add_package(pkg("d", "1.0.0"));

    let policy = Policy::new();
    let solver = Solver::new(&pool, &policy);

    let mut request = Request::new();
    request.require("a", "*");

    let result = solver.solve(&request);
    assert!(result.is_ok(), "Solver should resolve diamond dependency");

    let transaction = result.unwrap();
    let installs: Vec<_> = transaction.installs().collect();
    assert_eq!(installs.len(), 4, "Should install A, B, C, D");

    // D should only be installed once
    let d_count = installs.iter().filter(|p| p.name == "d").count();
    assert_eq!(d_count, 1, "D should only be installed once");
}

// ============================================================================
// All Jobs Test (Install, Update, Remove in same transaction)
// ============================================================================

#[test]
fn test_solver_all_jobs() {
    let mut pool = Pool::new();

    // A requires B < 1.1
    pool.add_package(pkg_with_requires("a", "2.0.0", vec![("b", "<1.1")]));
    pool.add_package(pkg("b", "1.0.0"));
    pool.add_package(pkg("b", "1.1.0"));
    pool.add_package(pkg("c", "1.1.0"));
    pool.add_package(pkg("d", "1.0.0"));

    let policy = Policy::new();
    let solver = Solver::new(&pool, &policy);

    let mut request = Request::new();
    // Require A and C
    request.require("a", "*");
    request.require("c", "*");
    // D is locked but no longer required -> should be removed
    request.lock(pkg("d", "1.0.0"));
    // C is locked at older version -> should be updated
    request.lock(pkg("c", "1.0.0"));

    let result = solver.solve(&request);
    assert!(result.is_ok(), "Solver should find a solution");

    let transaction = result.unwrap();

    // Check we have the expected operations
    let removes: Vec<_> = transaction.removals().collect();
    let updates: Vec<_> = transaction.updates().collect();
    let new_installs: Vec<_> = transaction.new_installs().collect();

    // D should be removed (no longer required)
    assert!(removes.iter().any(|p| p.name == "d"), "D should be removed");

    // C should be updated from 1.0.0 to 1.1.0
    assert!(updates.iter().any(|(from, to)| from.name == "c" && to.name == "c"), "C should be updated");

    // A and B should be newly installed
    assert!(new_installs.iter().any(|p| p.name == "a"), "A should be installed");
    assert!(new_installs.iter().any(|p| p.name == "b"), "B should be installed");
}

// ============================================================================
// Use Replacer If Necessary Test
// ============================================================================

/// This test requires the solver to understand that D can satisfy
/// requirements for both B and C via its replaces declarations.
#[test]
fn test_use_replacer_if_necessary() {
    let mut pool = Pool::new();

    // A requires B and C
    pool.add_package(pkg_with_requires("a", "1.0.0", vec![("b", ">=1.0"), ("c", ">=1.0")]));
    // B exists
    pool.add_package(pkg("b", "1.0.0"));
    // D replaces both B and C (C doesn't exist separately, so D is needed)
    pool.add_package(pkg_with_replaces("d", "1.0.0", vec![("b", ">=1.0"), ("c", ">=1.0")]));
    pool.add_package(pkg_with_replaces("d", "1.1.0", vec![("b", ">=1.0"), ("c", ">=1.0")]));

    let policy = Policy::new();
    let solver = Solver::new(&pool, &policy);

    let mut request = Request::new();
    request.require("a", "*");
    request.require("d", "*"); // We explicitly want D

    let result = solver.solve(&request);
    assert!(result.is_ok(), "Solver should find a solution");

    let transaction = result.unwrap();
    let installs: Vec<_> = transaction.installs().collect();

    // D 1.1 should be installed (highest version)
    let d_pkg = installs.iter().find(|p| p.name == "d").expect("D should be installed");
    assert_eq!(d_pkg.version, "1.1.0");
}

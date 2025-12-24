use std::collections::HashMap;

use super::rule::Literal;
use super::pool::PackageId;

/// A single decision record
#[derive(Debug, Clone, Copy)]
struct Decision {
    /// Whether the package is installed (true) or not (false)
    installed: bool,
    /// The decision level at which this was decided
    level: u32,
}

/// Tracks decisions made during SAT solving.
///
/// Each decision records:
/// - Whether a package is installed (+) or not installed (-)
/// - At what decision level it was decided
/// - Which rule caused the decision
#[derive(Debug)]
pub struct Decisions {
    /// Maps package ID to decision
    decision_map: HashMap<PackageId, Decision>,

    /// Queue of decisions in order made [(literal, rule_id)]
    decision_queue: Vec<(Literal, Option<u32>)>,

    /// Current decision level
    level: u32,
}

impl Decisions {
    /// Create a new empty decisions tracker
    pub fn new() -> Self {
        Self {
            decision_map: HashMap::new(),
            decision_queue: Vec::new(),
            level: 0,
        }
    }

    /// Get the current decision level
    pub fn level(&self) -> u32 {
        self.level
    }

    /// Increment the decision level
    pub fn increment_level(&mut self) {
        self.level += 1;
    }

    /// Set the decision level
    pub fn set_level(&mut self, level: u32) {
        self.level = level;
    }

    /// Make a decision at the current level
    ///
    /// Returns false if this conflicts with an existing decision
    pub fn decide(&mut self, literal: Literal, rule_id: Option<u32>) -> bool {
        let package_id = literal.unsigned_abs() as PackageId;
        let install = literal > 0;

        // Check for conflict
        if let Some(existing) = self.decision_map.get(&package_id) {
            if existing.installed != install {
                return false; // Conflict
            }
            return true; // Already decided the same way
        }

        // Record decision
        self.decision_map.insert(package_id, Decision {
            installed: install,
            level: self.level,
        });
        self.decision_queue.push((literal, rule_id));

        true
    }

    /// Check if a literal is satisfied by current decisions
    pub fn satisfied(&self, literal: Literal) -> bool {
        let package_id = literal.unsigned_abs() as PackageId;
        let want_installed = literal > 0;

        if let Some(decision) = self.decision_map.get(&package_id) {
            decision.installed == want_installed
        } else {
            false
        }
    }

    /// Check if a literal conflicts with current decisions
    pub fn conflict(&self, literal: Literal) -> bool {
        let package_id = literal.unsigned_abs() as PackageId;
        let want_installed = literal > 0;

        if let Some(decision) = self.decision_map.get(&package_id) {
            decision.installed != want_installed
        } else {
            false
        }
    }

    /// Check if a package has been decided (either way)
    pub fn decided(&self, package_id: PackageId) -> bool {
        self.decision_map.contains_key(&package_id)
    }

    /// Check if a package is undecided
    pub fn undecided(&self, package_id: PackageId) -> bool {
        !self.decided(package_id)
    }

    /// Check if a package was decided to be installed
    pub fn decided_install(&self, package_id: PackageId) -> bool {
        self.decision_map.get(&package_id).map(|d| d.installed).unwrap_or(false)
    }

    /// Check if a package was decided to not be installed
    pub fn decided_remove(&self, package_id: PackageId) -> bool {
        self.decision_map.get(&package_id).map(|d| !d.installed).unwrap_or(false)
    }

    /// Get the decision level for a literal/package
    pub fn decision_level(&self, literal: Literal) -> Option<u32> {
        let package_id = literal.unsigned_abs() as PackageId;
        self.decision_map.get(&package_id).map(|d| d.level)
    }

    /// Get the rule that caused a decision
    pub fn decision_rule(&self, literal: Literal) -> Option<u32> {
        let package_id = literal.unsigned_abs() as PackageId;

        // Find in queue
        for &(lit, rule_id) in &self.decision_queue {
            if lit.unsigned_abs() as PackageId == package_id {
                return rule_id;
            }
        }
        None
    }

    /// Revert all decisions at levels > target_level
    pub fn revert_to_level(&mut self, target_level: u32) {
        // Remove decisions from map
        self.decision_map.retain(|_, decision| {
            decision.level <= target_level
        });

        // Remove from queue
        self.decision_queue.retain(|(literal, _)| {
            let package_id = literal.unsigned_abs() as PackageId;
            self.decision_map.contains_key(&package_id)
        });

        self.level = target_level;
    }

    /// Get all packages decided to be installed
    pub fn installed_packages(&self) -> impl Iterator<Item = PackageId> + '_ {
        self.decision_map
            .iter()
            .filter(|(_, d)| d.installed)
            .map(|(&id, _)| id)
    }

    /// Get the decision queue
    pub fn queue(&self) -> &[(Literal, Option<u32>)] {
        &self.decision_queue
    }

    /// Get decisions at a specific level
    pub fn decisions_at_level(&self, level: u32) -> Vec<Literal> {
        self.decision_queue
            .iter()
            .filter_map(|&(literal, _)| {
                if self.decision_level(literal) == Some(level) {
                    Some(literal)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get the number of decisions
    pub fn len(&self) -> usize {
        self.decision_queue.len()
    }

    /// Check if no decisions have been made
    pub fn is_empty(&self) -> bool {
        self.decision_queue.is_empty()
    }

    /// Reset all decisions
    pub fn reset(&mut self) {
        self.decision_map.clear();
        self.decision_queue.clear();
        self.level = 0;
    }

    /// Get a snapshot of current decisions for debugging
    pub fn snapshot(&self) -> Vec<(PackageId, bool, u32)> {
        self.decision_map
            .iter()
            .map(|(&id, decision)| {
                (id, decision.installed, decision.level)
            })
            .collect()
    }
}

impl Default for Decisions {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decisions_new() {
        let decisions = Decisions::new();
        assert_eq!(decisions.level(), 0);
        assert!(decisions.is_empty());
    }

    #[test]
    fn test_decisions_decide() {
        let mut decisions = Decisions::new();

        // Decide to install package 1
        assert!(decisions.decide(1, Some(0)));
        assert!(decisions.satisfied(1));
        assert!(!decisions.satisfied(-1));
        assert!(decisions.decided_install(1));

        // Decide to not install package 2
        assert!(decisions.decide(-2, Some(1)));
        assert!(decisions.satisfied(-2));
        assert!(!decisions.satisfied(2));
        assert!(decisions.decided_remove(2));
    }

    #[test]
    fn test_decisions_conflict() {
        let mut decisions = Decisions::new();

        decisions.decide(1, None);

        // Trying to decide opposite should fail
        assert!(!decisions.decide(-1, None));

        // Check conflict detection
        assert!(decisions.conflict(-1));
        assert!(!decisions.conflict(1));
    }

    #[test]
    fn test_decisions_levels() {
        let mut decisions = Decisions::new();
        decisions.increment_level(); // Level 1

        decisions.decide(1, None);
        assert_eq!(decisions.decision_level(1), Some(1));

        decisions.increment_level(); // Level 2
        decisions.decide(2, None);
        assert_eq!(decisions.decision_level(2), Some(2));
    }

    #[test]
    fn test_decisions_revert() {
        let mut decisions = Decisions::new();

        decisions.increment_level();
        decisions.decide(1, None);

        decisions.increment_level();
        decisions.decide(2, None);

        decisions.increment_level();
        decisions.decide(3, None);

        // Revert to level 1
        decisions.revert_to_level(1);

        assert!(decisions.decided(1));
        assert!(!decisions.decided(2));
        assert!(!decisions.decided(3));
        assert_eq!(decisions.level(), 1);
    }

    #[test]
    fn test_decisions_installed_packages() {
        let mut decisions = Decisions::new();
        decisions.decide(1, None);
        decisions.decide(-2, None);
        decisions.decide(3, None);

        let installed: Vec<_> = decisions.installed_packages().collect();
        assert_eq!(installed.len(), 2);
        assert!(installed.contains(&1));
        assert!(installed.contains(&3));
        assert!(!installed.contains(&2));
    }

    #[test]
    fn test_decisions_undecided() {
        let mut decisions = Decisions::new();
        decisions.decide(1, None);

        assert!(!decisions.undecided(1));
        assert!(decisions.undecided(2));
    }

    #[test]
    fn test_decisions_decision_rule() {
        let mut decisions = Decisions::new();
        decisions.decide(1, Some(42));
        decisions.decide(2, None);

        assert_eq!(decisions.decision_rule(1), Some(42));
        assert_eq!(decisions.decision_rule(2), None);
    }
}

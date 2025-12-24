//! Single version constraint implementation

use std::fmt;
use thiserror::Error;

use super::{Bound, ConstraintInterface, Operator};

#[derive(Error, Debug)]
pub enum ConstraintError {
    #[error("Invalid operator \"{operator}\", expected one of: {expected}")]
    InvalidOperator { operator: String, expected: String },
}

/// A single version constraint (e.g., ">= 1.0.0")
#[derive(Debug, Clone)]
pub struct Constraint {
    operator: Operator,
    version: String,
    pretty_string: Option<String>,
    lower_bound: Option<Bound>,
    upper_bound: Option<Bound>,
}

impl Constraint {
    /// Create a new constraint
    pub fn new(operator: Operator, version: String) -> Result<Self, ConstraintError> {
        Ok(Constraint {
            operator,
            version,
            pretty_string: None,
            lower_bound: None,
            upper_bound: None,
        })
    }

    /// Create a constraint from operator string
    pub fn from_str(operator: &str, version: String) -> Result<Self, ConstraintError> {
        let op = Operator::from_str(operator).map_err(|_| ConstraintError::InvalidOperator {
            operator: operator.to_string(),
            expected: Operator::supported_operators().join(", "),
        })?;
        Self::new(op, version)
    }

    /// Get the version
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Get the operator
    pub fn operator(&self) -> Operator {
        self.operator
    }

    /// Match against another single constraint
    pub fn match_specific(&self, provider: &Constraint, compare_branches: bool) -> bool {
        let is_equal_op = self.operator == Operator::Equal;
        let is_non_equal_op = self.operator == Operator::NotEqual;
        let is_provider_equal_op = provider.operator == Operator::Equal;
        let is_provider_non_equal_op = provider.operator == Operator::NotEqual;

        // != operator handling
        if is_non_equal_op || is_provider_non_equal_op {
            if is_non_equal_op
                && !is_provider_non_equal_op
                && !is_provider_equal_op
                && provider.version.starts_with("dev-")
            {
                return false;
            }

            if is_provider_non_equal_op
                && !is_non_equal_op
                && !is_equal_op
                && self.version.starts_with("dev-")
            {
                return false;
            }

            if !is_equal_op && !is_provider_equal_op {
                return true;
            }

            return self.version_compare(&provider.version, &self.version, Operator::NotEqual, compare_branches);
        }

        // Same direction comparisons always have a solution (both < or both >)
        // Check if both operators are in the same "direction" (both less-than-ish or both greater-than-ish)
        let self_direction = match self.operator {
            Operator::LessThan | Operator::LessThanOrEqual => Some("less"),
            Operator::GreaterThan | Operator::GreaterThanOrEqual => Some("greater"),
            _ => None,
        };
        let provider_direction = match provider.operator {
            Operator::LessThan | Operator::LessThanOrEqual => Some("less"),
            Operator::GreaterThan | Operator::GreaterThanOrEqual => Some("greater"),
            _ => None,
        };

        if self_direction.is_some() && self_direction == provider_direction {
            return !(self.version.starts_with("dev-") || provider.version.starts_with("dev-"));
        }

        let (version1, version2, operator) = if is_equal_op {
            (&self.version, &provider.version, provider.operator)
        } else {
            (&provider.version, &self.version, self.operator)
        };

        if self.version_compare(version1, version2, operator, compare_branches) {
            // Special case: opposite direction operators with no intersection
            // e.g., require >= 1.0 and provide < 1.0 should NOT match
            // But require >= 2 and provide <= 2 SHOULD match (they meet at 2)
            if !is_equal_op && !is_provider_equal_op {
                // Check if operators are opposite directions
                let opposite_directions = self_direction.is_some()
                    && provider_direction.is_some()
                    && self_direction != provider_direction;

                if opposite_directions {
                    // If same version but opposite directions, check if they can meet
                    if php_version_compare(&provider.version, &self.version, "==") {
                        // Same version - they only intersect if both are inclusive
                        let self_inclusive = self.operator == Operator::LessThanOrEqual
                            || self.operator == Operator::GreaterThanOrEqual;
                        let provider_inclusive = provider.operator == Operator::LessThanOrEqual
                            || provider.operator == Operator::GreaterThanOrEqual;
                        return self_inclusive && provider_inclusive;
                    }
                    // Different versions - opposite directions always intersect somewhere
                    return true;
                }
            }
            return true;
        }

        false
    }

    /// Compare two versions with an operator
    pub fn version_compare(
        &self,
        a: &str,
        b: &str,
        operator: Operator,
        compare_branches: bool,
    ) -> bool {
        let a_is_branch = a.starts_with("dev-");
        let b_is_branch = b.starts_with("dev-");

        if operator == Operator::NotEqual && (a_is_branch || b_is_branch) {
            return a != b;
        }

        if a_is_branch && b_is_branch {
            return operator == Operator::Equal && a == b;
        }

        // When branches are not comparable, dev branches never match anything
        if !compare_branches && (a_is_branch || b_is_branch) {
            return false;
        }

        php_version_compare(a, b, operator.as_str())
    }

    fn extract_bounds(&mut self) {
        if self.lower_bound.is_some() {
            return;
        }

        // Branches have infinite bounds
        if self.version.starts_with("dev-") {
            self.lower_bound = Some(Bound::zero());
            self.upper_bound = Some(Bound::positive_infinity());
            return;
        }

        match self.operator {
            Operator::Equal => {
                self.lower_bound = Some(Bound::new(self.version.clone(), true));
                self.upper_bound = Some(Bound::new(self.version.clone(), true));
            }
            Operator::LessThan => {
                self.lower_bound = Some(Bound::zero());
                self.upper_bound = Some(Bound::new(self.version.clone(), false));
            }
            Operator::LessThanOrEqual => {
                self.lower_bound = Some(Bound::zero());
                self.upper_bound = Some(Bound::new(self.version.clone(), true));
            }
            Operator::GreaterThan => {
                self.lower_bound = Some(Bound::new(self.version.clone(), false));
                self.upper_bound = Some(Bound::positive_infinity());
            }
            Operator::GreaterThanOrEqual => {
                self.lower_bound = Some(Bound::new(self.version.clone(), true));
                self.upper_bound = Some(Bound::positive_infinity());
            }
            Operator::NotEqual => {
                self.lower_bound = Some(Bound::zero());
                self.upper_bound = Some(Bound::positive_infinity());
            }
        }
    }
}

impl ConstraintInterface for Constraint {
    fn matches(&self, other: &dyn ConstraintInterface) -> bool {
        // If other is a single Constraint, use match_specific
        if let Some((op, ver)) = other.as_constraint() {
            if let Ok(provider) = Constraint::new(*op, ver.to_string()) {
                return self.match_specific(&provider, false);
            }
        }

        // If other is MatchAllConstraint
        if other.is_match_all() {
            return true;
        }

        // If other is MatchNoneConstraint
        if other.is_match_none() {
            return false;
        }

        // For MultiConstraint, delegate to its matches
        other.matches(self)
    }

    fn lower_bound(&self) -> Bound {
        let mut s = self.clone();
        s.extract_bounds();
        s.lower_bound.unwrap()
    }

    fn upper_bound(&self) -> Bound {
        let mut s = self.clone();
        s.extract_bounds();
        s.upper_bound.unwrap()
    }

    fn pretty_string(&self) -> String {
        self.pretty_string
            .clone()
            .unwrap_or_else(|| self.to_string())
    }

    fn set_pretty_string(&mut self, pretty: Option<String>) {
        self.pretty_string = pretty;
    }

    fn clone_box(&self) -> Box<dyn ConstraintInterface> {
        Box::new(self.clone())
    }

    fn as_constraint(&self) -> Option<(&Operator, &str)> {
        Some((&self.operator, &self.version))
    }
}

impl fmt::Display for Constraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.operator, self.version)
    }
}

/// PHP-compatible version_compare
pub fn php_version_compare(a: &str, b: &str, operator: &str) -> bool {
    let cmp = compare_versions(a, b);

    match operator {
        "==" | "=" => cmp == std::cmp::Ordering::Equal,
        "!=" | "<>" => cmp != std::cmp::Ordering::Equal,
        "<" => cmp == std::cmp::Ordering::Less,
        "<=" => cmp != std::cmp::Ordering::Greater,
        ">" => cmp == std::cmp::Ordering::Greater,
        ">=" => cmp != std::cmp::Ordering::Less,
        _ => false,
    }
}

/// Compare two version strings (PHP version_compare compatible)
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let a_parts = split_version(a);
    let b_parts = split_version(b);

    let max_len = std::cmp::max(a_parts.len(), b_parts.len());

    for i in 0..max_len {
        let a_part = a_parts.get(i).map(|s| s.as_str()).unwrap_or("");
        let b_part = b_parts.get(i).map(|s| s.as_str()).unwrap_or("");

        let cmp = compare_part(a_part, b_part);
        if cmp != std::cmp::Ordering::Equal {
            return cmp;
        }
    }

    std::cmp::Ordering::Equal
}

fn split_version(version: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut prev_type: Option<CharType> = None;

    for c in version.chars() {
        let current_type = if c.is_ascii_digit() {
            CharType::Digit
        } else if c.is_alphabetic() {
            CharType::Alpha
        } else {
            CharType::Separator
        };

        if current_type == CharType::Separator {
            if !current.is_empty() {
                parts.push(current.clone());
                current.clear();
            }
            prev_type = None;
            continue;
        }

        if prev_type.is_some() && prev_type != Some(current_type) {
            if !current.is_empty() {
                parts.push(current.clone());
                current.clear();
            }
        }

        current.push(c);
        prev_type = Some(current_type);
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
}

#[derive(Clone, Copy, PartialEq)]
enum CharType {
    Digit,
    Alpha,
    Separator,
}

fn compare_part(a: &str, b: &str) -> std::cmp::Ordering {
    // Check if both are numeric
    let a_num = a.parse::<i64>();
    let b_num = b.parse::<i64>();

    match (a_num, b_num) {
        (Ok(an), Ok(bn)) => an.cmp(&bn),
        (Ok(_), Err(_)) => std::cmp::Ordering::Greater,
        (Err(_), Ok(_)) => std::cmp::Ordering::Less,
        (Err(_), Err(_)) => {
            // Both are strings - use special ordering
            let a_order = special_order(a);
            let b_order = special_order(b);
            a_order.cmp(&b_order)
        }
    }
}

fn special_order(s: &str) -> i32 {
    match s.to_lowercase().as_str() {
        "dev" => 0,
        "alpha" | "a" => 1,
        "beta" | "b" => 2,
        "rc" => 3,
        "" | "stable" => 4,
        "patch" | "pl" | "p" => 5,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constraint_creation() {
        let c = Constraint::new(Operator::Equal, "1.0.0".to_string()).unwrap();
        assert_eq!(c.version(), "1.0.0");
        assert_eq!(c.operator(), Operator::Equal);
    }

    #[test]
    fn test_constraint_display() {
        let c = Constraint::new(Operator::GreaterThanOrEqual, "1.0.0".to_string()).unwrap();
        assert_eq!(c.to_string(), ">= 1.0.0");
    }

    #[test]
    fn test_version_compare() {
        assert!(php_version_compare("1.0.0", "1.0.0", "=="));
        assert!(php_version_compare("2.0.0", "1.0.0", ">"));
        assert!(php_version_compare("1.0.0", "2.0.0", "<"));
        assert!(php_version_compare("1.0.0", "1.0.0", ">="));
        assert!(php_version_compare("1.0.0", "1.0.0", "<="));
        assert!(!php_version_compare("1.0.0", "1.0.0", "!="));
    }

    #[test]
    fn test_match_specific() {
        let c1 = Constraint::new(Operator::GreaterThan, "1.0.0".to_string()).unwrap();
        let c2 = Constraint::new(Operator::Equal, "2.0.0".to_string()).unwrap();
        assert!(c1.match_specific(&c2, false));

        let c3 = Constraint::new(Operator::Equal, "0.5.0".to_string()).unwrap();
        assert!(!c1.match_specific(&c3, false));
    }

    #[test]
    fn test_bounds() {
        let c = Constraint::new(Operator::GreaterThanOrEqual, "1.0.0".to_string()).unwrap();
        assert_eq!(c.lower_bound().version(), "1.0.0");
        assert!(c.lower_bound().is_inclusive());
        assert!(c.upper_bound().is_positive_infinity());
    }

    #[test]
    fn test_equal_equal_match() {
        let c1 = Constraint::new(Operator::Equal, "1.0.0.0".to_string()).unwrap();
        let c2 = Constraint::new(Operator::Equal, "1.0.0.0".to_string()).unwrap();
        assert!(c1.match_specific(&c2, false));
    }

    // Helper function to test constraint matching
    fn test_match(req_op: Operator, req_ver: &str, prov_op: Operator, prov_ver: &str) -> bool {
        let require = Constraint::new(req_op, req_ver.to_string()).unwrap();
        let provide = Constraint::new(prov_op, prov_ver.to_string()).unwrap();
        require.match_specific(&provide, false)
    }

    #[test]
    fn test_version_match_succeeds_equal() {
        // == matches
        assert!(test_match(Operator::Equal, "2", Operator::Equal, "2"));
        assert!(test_match(Operator::Equal, "2", Operator::LessThan, "3"));
        assert!(test_match(Operator::Equal, "2", Operator::LessThanOrEqual, "2"));
        assert!(test_match(Operator::Equal, "2", Operator::LessThanOrEqual, "3"));
        assert!(test_match(Operator::Equal, "2", Operator::GreaterThanOrEqual, "1"));
        assert!(test_match(Operator::Equal, "2", Operator::GreaterThanOrEqual, "2"));
        assert!(test_match(Operator::Equal, "2", Operator::GreaterThan, "1"));
        assert!(test_match(Operator::Equal, "2", Operator::NotEqual, "1"));
        assert!(test_match(Operator::Equal, "2", Operator::NotEqual, "3"));
    }

    #[test]
    fn test_version_match_succeeds_less_than() {
        // < matches
        assert!(test_match(Operator::LessThan, "2", Operator::Equal, "1"));
        assert!(test_match(Operator::LessThan, "2", Operator::LessThan, "1"));
        assert!(test_match(Operator::LessThan, "2", Operator::LessThan, "2"));
        assert!(test_match(Operator::LessThan, "2", Operator::LessThan, "3"));
        assert!(test_match(Operator::LessThan, "2", Operator::LessThanOrEqual, "1"));
        assert!(test_match(Operator::LessThan, "2", Operator::LessThanOrEqual, "2"));
        assert!(test_match(Operator::LessThan, "2", Operator::LessThanOrEqual, "3"));
        assert!(test_match(Operator::LessThan, "2", Operator::GreaterThanOrEqual, "1"));
        assert!(test_match(Operator::LessThan, "2", Operator::GreaterThan, "1"));
        assert!(test_match(Operator::LessThan, "2", Operator::NotEqual, "1"));
        assert!(test_match(Operator::LessThan, "2", Operator::NotEqual, "2"));
        assert!(test_match(Operator::LessThan, "2", Operator::NotEqual, "3"));
    }

    #[test]
    fn test_version_match_succeeds_less_than_or_equal() {
        // <= matches
        assert!(test_match(Operator::LessThanOrEqual, "2", Operator::Equal, "1"));
        assert!(test_match(Operator::LessThanOrEqual, "2", Operator::Equal, "2"));
        assert!(test_match(Operator::LessThanOrEqual, "2", Operator::LessThan, "1"));
        assert!(test_match(Operator::LessThanOrEqual, "2", Operator::LessThan, "2"));
        assert!(test_match(Operator::LessThanOrEqual, "2", Operator::LessThan, "3"));
        assert!(test_match(Operator::LessThanOrEqual, "2", Operator::LessThanOrEqual, "1"));
        assert!(test_match(Operator::LessThanOrEqual, "2", Operator::LessThanOrEqual, "2"));
        assert!(test_match(Operator::LessThanOrEqual, "2", Operator::LessThanOrEqual, "3"));
        assert!(test_match(Operator::LessThanOrEqual, "2", Operator::GreaterThanOrEqual, "1"));
        assert!(test_match(Operator::LessThanOrEqual, "2", Operator::GreaterThanOrEqual, "2"));
        assert!(test_match(Operator::LessThanOrEqual, "2", Operator::GreaterThan, "1"));
        assert!(test_match(Operator::LessThanOrEqual, "2", Operator::NotEqual, "1"));
        assert!(test_match(Operator::LessThanOrEqual, "2", Operator::NotEqual, "2"));
        assert!(test_match(Operator::LessThanOrEqual, "2", Operator::NotEqual, "3"));
    }

    #[test]
    fn test_version_match_succeeds_greater_than_or_equal() {
        // >= matches
        assert!(test_match(Operator::GreaterThanOrEqual, "2", Operator::Equal, "2"));
        assert!(test_match(Operator::GreaterThanOrEqual, "2", Operator::Equal, "3"));
        assert!(test_match(Operator::GreaterThanOrEqual, "2", Operator::LessThan, "3"));
        assert!(test_match(Operator::GreaterThanOrEqual, "2", Operator::LessThanOrEqual, "2"));
        assert!(test_match(Operator::GreaterThanOrEqual, "2", Operator::LessThanOrEqual, "3"));
        assert!(test_match(Operator::GreaterThanOrEqual, "2", Operator::GreaterThanOrEqual, "1"));
        assert!(test_match(Operator::GreaterThanOrEqual, "2", Operator::GreaterThanOrEqual, "2"));
        assert!(test_match(Operator::GreaterThanOrEqual, "2", Operator::GreaterThanOrEqual, "3"));
        assert!(test_match(Operator::GreaterThanOrEqual, "2", Operator::GreaterThan, "1"));
        assert!(test_match(Operator::GreaterThanOrEqual, "2", Operator::GreaterThan, "2"));
        assert!(test_match(Operator::GreaterThanOrEqual, "2", Operator::GreaterThan, "3"));
        assert!(test_match(Operator::GreaterThanOrEqual, "2", Operator::NotEqual, "1"));
        assert!(test_match(Operator::GreaterThanOrEqual, "2", Operator::NotEqual, "2"));
        assert!(test_match(Operator::GreaterThanOrEqual, "2", Operator::NotEqual, "3"));
    }

    #[test]
    fn test_version_match_succeeds_greater_than() {
        // > matches
        assert!(test_match(Operator::GreaterThan, "2", Operator::Equal, "3"));
        assert!(test_match(Operator::GreaterThan, "2", Operator::LessThan, "3"));
        assert!(test_match(Operator::GreaterThan, "2", Operator::LessThanOrEqual, "3"));
        assert!(test_match(Operator::GreaterThan, "2", Operator::GreaterThanOrEqual, "1"));
        assert!(test_match(Operator::GreaterThan, "2", Operator::GreaterThanOrEqual, "2"));
        assert!(test_match(Operator::GreaterThan, "2", Operator::GreaterThanOrEqual, "3"));
        assert!(test_match(Operator::GreaterThan, "2", Operator::GreaterThan, "1"));
        assert!(test_match(Operator::GreaterThan, "2", Operator::GreaterThan, "2"));
        assert!(test_match(Operator::GreaterThan, "2", Operator::GreaterThan, "3"));
        assert!(test_match(Operator::GreaterThan, "2", Operator::NotEqual, "1"));
        assert!(test_match(Operator::GreaterThan, "2", Operator::NotEqual, "2"));
        assert!(test_match(Operator::GreaterThan, "2", Operator::NotEqual, "3"));
    }

    #[test]
    fn test_version_match_succeeds_not_equal() {
        // != matches
        assert!(test_match(Operator::NotEqual, "2", Operator::NotEqual, "1"));
        assert!(test_match(Operator::NotEqual, "2", Operator::NotEqual, "2"));
        assert!(test_match(Operator::NotEqual, "2", Operator::NotEqual, "3"));
        assert!(test_match(Operator::NotEqual, "2", Operator::Equal, "1"));
        assert!(test_match(Operator::NotEqual, "2", Operator::Equal, "3"));
        assert!(test_match(Operator::NotEqual, "2", Operator::LessThan, "1"));
        assert!(test_match(Operator::NotEqual, "2", Operator::LessThan, "2"));
        assert!(test_match(Operator::NotEqual, "2", Operator::LessThan, "3"));
        assert!(test_match(Operator::NotEqual, "2", Operator::LessThanOrEqual, "1"));
        assert!(test_match(Operator::NotEqual, "2", Operator::LessThanOrEqual, "2"));
        assert!(test_match(Operator::NotEqual, "2", Operator::LessThanOrEqual, "3"));
        assert!(test_match(Operator::NotEqual, "2", Operator::GreaterThanOrEqual, "1"));
        assert!(test_match(Operator::NotEqual, "2", Operator::GreaterThanOrEqual, "2"));
        assert!(test_match(Operator::NotEqual, "2", Operator::GreaterThanOrEqual, "3"));
        assert!(test_match(Operator::NotEqual, "2", Operator::GreaterThan, "1"));
        assert!(test_match(Operator::NotEqual, "2", Operator::GreaterThan, "2"));
        assert!(test_match(Operator::NotEqual, "2", Operator::GreaterThan, "3"));
    }

    #[test]
    fn test_version_match_succeeds_branches() {
        // Branch names
        assert!(test_match(Operator::Equal, "dev-foo-bar", Operator::Equal, "dev-foo-bar"));
        assert!(test_match(Operator::Equal, "dev-events+issue-17", Operator::Equal, "dev-events+issue-17"));
        assert!(test_match(Operator::Equal, "dev-foo-bar", Operator::NotEqual, "dev-foo-xyz"));
        assert!(test_match(Operator::NotEqual, "dev-foo-bar", Operator::NotEqual, "dev-foo-xyz"));
    }

    #[test]
    fn test_version_match_succeeds_numbers_vs_branches() {
        // Numbers vs branches
        assert!(test_match(Operator::Equal, "0.12", Operator::NotEqual, "dev-foo"));
        assert!(test_match(Operator::LessThan, "0.12", Operator::NotEqual, "dev-foo"));
        assert!(test_match(Operator::LessThanOrEqual, "0.12", Operator::NotEqual, "dev-foo"));
        assert!(test_match(Operator::GreaterThanOrEqual, "0.12", Operator::NotEqual, "dev-foo"));
        assert!(test_match(Operator::GreaterThan, "0.12", Operator::NotEqual, "dev-foo"));
        assert!(test_match(Operator::NotEqual, "0.12", Operator::Equal, "dev-foo"));
        assert!(test_match(Operator::NotEqual, "0.12", Operator::NotEqual, "dev-foo"));
    }

    #[test]
    fn test_version_match_fails_equal() {
        // == fails
        assert!(!test_match(Operator::Equal, "2", Operator::Equal, "1"));
        assert!(!test_match(Operator::Equal, "2", Operator::Equal, "3"));
        assert!(!test_match(Operator::Equal, "2", Operator::LessThan, "1"));
        assert!(!test_match(Operator::Equal, "2", Operator::LessThan, "2"));
        assert!(!test_match(Operator::Equal, "2", Operator::LessThanOrEqual, "1"));
        assert!(!test_match(Operator::Equal, "2", Operator::GreaterThanOrEqual, "3"));
        assert!(!test_match(Operator::Equal, "2", Operator::GreaterThan, "2"));
        assert!(!test_match(Operator::Equal, "2", Operator::GreaterThan, "3"));
        assert!(!test_match(Operator::Equal, "2", Operator::NotEqual, "2"));
    }

    #[test]
    fn test_version_match_fails_less_than() {
        // < fails
        assert!(!test_match(Operator::LessThan, "2", Operator::Equal, "2"));
        assert!(!test_match(Operator::LessThan, "2", Operator::Equal, "3"));
        assert!(!test_match(Operator::LessThan, "2", Operator::GreaterThanOrEqual, "2"));
        assert!(!test_match(Operator::LessThan, "2", Operator::GreaterThanOrEqual, "3"));
        assert!(!test_match(Operator::LessThan, "2", Operator::GreaterThan, "2"));
        assert!(!test_match(Operator::LessThan, "2", Operator::GreaterThan, "3"));
    }

    #[test]
    fn test_version_match_fails_less_than_or_equal() {
        // <= fails
        assert!(!test_match(Operator::LessThanOrEqual, "2", Operator::Equal, "3"));
        assert!(!test_match(Operator::LessThanOrEqual, "2", Operator::GreaterThanOrEqual, "3"));
        assert!(!test_match(Operator::LessThanOrEqual, "2", Operator::GreaterThan, "2"));
        assert!(!test_match(Operator::LessThanOrEqual, "2", Operator::GreaterThan, "3"));
    }

    #[test]
    fn test_version_match_fails_greater_than_or_equal() {
        // >= fails
        assert!(!test_match(Operator::GreaterThanOrEqual, "2", Operator::Equal, "1"));
        assert!(!test_match(Operator::GreaterThanOrEqual, "2", Operator::LessThan, "1"));
        assert!(!test_match(Operator::GreaterThanOrEqual, "2", Operator::LessThan, "2"));
        assert!(!test_match(Operator::GreaterThanOrEqual, "2", Operator::LessThanOrEqual, "1"));
    }

    #[test]
    fn test_version_match_fails_greater_than() {
        // > fails
        assert!(!test_match(Operator::GreaterThan, "2", Operator::Equal, "1"));
        assert!(!test_match(Operator::GreaterThan, "2", Operator::Equal, "2"));
        assert!(!test_match(Operator::GreaterThan, "2", Operator::LessThan, "1"));
        assert!(!test_match(Operator::GreaterThan, "2", Operator::LessThan, "2"));
        assert!(!test_match(Operator::GreaterThan, "2", Operator::LessThanOrEqual, "1"));
        assert!(!test_match(Operator::GreaterThan, "2", Operator::LessThanOrEqual, "2"));
    }

    #[test]
    fn test_version_match_fails_not_equal() {
        // != fails
        assert!(!test_match(Operator::NotEqual, "2", Operator::Equal, "2"));
    }

    #[test]
    fn test_version_match_fails_branches() {
        // Different branch names
        assert!(!test_match(Operator::Equal, "dev-foo-bar", Operator::Equal, "dev-foo-xyz"));
        assert!(!test_match(Operator::Equal, "dev-foo-bar", Operator::LessThan, "dev-foo-xyz"));
        assert!(!test_match(Operator::Equal, "dev-foo-bar", Operator::LessThanOrEqual, "dev-foo-xyz"));
        assert!(!test_match(Operator::Equal, "dev-foo-bar", Operator::GreaterThanOrEqual, "dev-foo-xyz"));
        assert!(!test_match(Operator::Equal, "dev-foo-bar", Operator::GreaterThan, "dev-foo-xyz"));

        // Same branch - non-equal operators always fail
        assert!(!test_match(Operator::Equal, "dev-foo-bar", Operator::NotEqual, "dev-foo-bar"));
        assert!(!test_match(Operator::LessThan, "dev-foo-bar", Operator::Equal, "dev-foo-bar"));
        assert!(!test_match(Operator::LessThan, "dev-foo-bar", Operator::LessThan, "dev-foo-bar"));
        assert!(!test_match(Operator::GreaterThan, "dev-foo-bar", Operator::Equal, "dev-foo-bar"));
        assert!(!test_match(Operator::GreaterThan, "dev-foo-bar", Operator::GreaterThan, "dev-foo-bar"));
    }

    #[test]
    fn test_version_match_fails_numbers_vs_branches() {
        // Branch vs number, not comparable so mostly false
        assert!(!test_match(Operator::Equal, "0.12", Operator::Equal, "dev-foo"));
        assert!(!test_match(Operator::Equal, "0.12", Operator::LessThan, "dev-foo"));
        assert!(!test_match(Operator::Equal, "0.12", Operator::LessThanOrEqual, "dev-foo"));
        assert!(!test_match(Operator::Equal, "0.12", Operator::GreaterThanOrEqual, "dev-foo"));
        assert!(!test_match(Operator::Equal, "0.12", Operator::GreaterThan, "dev-foo"));

        assert!(!test_match(Operator::LessThan, "0.12", Operator::Equal, "dev-foo"));
        assert!(!test_match(Operator::LessThan, "0.12", Operator::LessThan, "dev-foo"));
        assert!(!test_match(Operator::LessThan, "0.12", Operator::LessThanOrEqual, "dev-foo"));
        assert!(!test_match(Operator::LessThan, "0.12", Operator::GreaterThanOrEqual, "dev-foo"));
        assert!(!test_match(Operator::LessThan, "0.12", Operator::GreaterThan, "dev-foo"));

        assert!(!test_match(Operator::GreaterThan, "0.12", Operator::Equal, "dev-foo"));
        assert!(!test_match(Operator::GreaterThan, "0.12", Operator::LessThan, "dev-foo"));
        assert!(!test_match(Operator::GreaterThan, "0.12", Operator::LessThanOrEqual, "dev-foo"));
        assert!(!test_match(Operator::GreaterThan, "0.12", Operator::GreaterThanOrEqual, "dev-foo"));
        assert!(!test_match(Operator::GreaterThan, "0.12", Operator::GreaterThan, "dev-foo"));
    }

    #[test]
    fn test_bounds_comprehensive() {
        // Equal bounds
        let c = Constraint::new(Operator::Equal, "1.0.0.0".to_string()).unwrap();
        assert_eq!(c.lower_bound().version(), "1.0.0.0");
        assert!(c.lower_bound().is_inclusive());
        assert_eq!(c.upper_bound().version(), "1.0.0.0");
        assert!(c.upper_bound().is_inclusive());

        // Less than bounds
        let c = Constraint::new(Operator::LessThan, "1.0.0.0".to_string()).unwrap();
        assert!(c.lower_bound().is_zero());
        assert_eq!(c.upper_bound().version(), "1.0.0.0");
        assert!(!c.upper_bound().is_inclusive());

        // Less than or equal bounds
        let c = Constraint::new(Operator::LessThanOrEqual, "1.0.0.0".to_string()).unwrap();
        assert!(c.lower_bound().is_zero());
        assert_eq!(c.upper_bound().version(), "1.0.0.0");
        assert!(c.upper_bound().is_inclusive());

        // Greater than bounds
        let c = Constraint::new(Operator::GreaterThan, "1.0.0.0".to_string()).unwrap();
        assert_eq!(c.lower_bound().version(), "1.0.0.0");
        assert!(!c.lower_bound().is_inclusive());
        assert!(c.upper_bound().is_positive_infinity());

        // Greater than or equal bounds
        let c = Constraint::new(Operator::GreaterThanOrEqual, "1.0.0.0".to_string()).unwrap();
        assert_eq!(c.lower_bound().version(), "1.0.0.0");
        assert!(c.lower_bound().is_inclusive());
        assert!(c.upper_bound().is_positive_infinity());

        // Not equal bounds (infinite range)
        let c = Constraint::new(Operator::NotEqual, "1.0.0.0".to_string()).unwrap();
        assert!(c.lower_bound().is_zero());
        assert!(c.upper_bound().is_positive_infinity());

        // Dev branch bounds (infinite range)
        let c = Constraint::new(Operator::GreaterThanOrEqual, "dev-feature-branch".to_string()).unwrap();
        assert!(c.lower_bound().is_zero());
        assert!(c.upper_bound().is_positive_infinity());
    }
}

//! `compute_ctor_renames`: pure union-constructor rename computation.
//!
//! Background: the Lean backend emits each Sail union as an inductive
//! type whose constructors live in the same Lean namespace as type
//! names. When a constructor carries the *same name* as some type
//! (e.g. `union U { U | V }` alongside `struct U`, or two unions both
//! defining `None`), Lean rejects the file. The printer therefore
//! renames the offending constructors; this function computes that
//! renaming as a **pure, deterministic** map `original -> fresh`.
//!
//! Rules:
//! - Only union constructors are renamed (enums map to Lean `inductive`
//!   members the same way, but the Lean backend historically only
//!   renames union ctors; callers may pass enum ctors in too — the
//!   function does not care).
//! - A ctor needs a rename iff its name is in `type_names` (collision
//!   with a type) or it is defined by more than one owner (two unions
//!   defining `Some`).
//! - Fresh names append `_ctor`, then `_ctor2`, `_ctor3`, … until the
//!   name is free of `type_names`, all input ctor names, and all
//!   previously assigned fresh names.
//! - Iteration order is sorted, so output is deterministic for the same
//!   input regardless of `HashMap` order. Only colliding ctors appear
//!   as keys; non-colliding ctors map to themselves implicitly.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

/// Compute constructor renames.
///
/// * `type_names`: every type-level name in scope (structs, enums,
///   unions, abbrevs — anything that becomes a Lean top-level name).
/// * `union_ctors`: `(owner_type, ctor_names)` per union (or enum).
///
/// Returns `original_ctor_name -> fresh_name` for colliding ctors only.
/// Each original name maps to exactly one fresh name even if several
/// owners define it (first sorted owner wins the mapping; duplicates
/// across owners are still all renamed).
#[must_use]
pub fn compute_ctor_renames(
    type_names: &BTreeSet<String>,
    union_ctors: &[(String, Vec<String>)],
) -> BTreeMap<String, String> {
    // Count owners per ctor to detect multi-owner collisions.
    let mut owners: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (owner, ctors) in union_ctors {
        for c in ctors {
            owners.entry(c.as_str()).or_default().push(owner.as_str());
        }
    }

    // Names taken from the start: all types + all ctor names.
    let taken: BTreeSet<&str> = type_names
        .iter()
        .map(String::as_str)
        .chain(owners.keys().copied())
        .collect();

    let mut renames: BTreeMap<String, String> = BTreeMap::new();
    let mut used: BTreeSet<String> = taken.iter().map(ToString::to_string).collect();

    for (ctor, owner_list) in &owners {
        let collides_type = type_names.contains(*ctor);
        let collides_multi = owner_list.len() > 1;
        if !collides_type && !collides_multi {
            continue;
        }
        let fresh = fresh_name(ctor, &used);
        used.insert(fresh.clone());
        renames.insert((*ctor).to_owned(), fresh);
    }
    renames
}

/// Smallest `{base}_ctor{,2,3,…}` not in `used`.
fn fresh_name(base: &str, used: &BTreeSet<String>) -> String {
    let mut cand = format!("{base}_ctor");
    if !used.contains(&cand) {
        return cand;
    }
    for n in 2_u32.. {
        cand = format!("{base}_ctor{n}");
        if !used.contains(&cand) {
            return cand;
        }
    }
    unreachable!("u32 suffix space exhausted")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(xs: &[&str]) -> BTreeSet<String> {
        xs.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn no_collision_no_renames() {
        let out = compute_ctor_renames(
            &set(&["U", "V"]),
            &[("U".to_owned(), vec!["A".to_owned(), "B".to_owned()])],
        );
        assert!(out.is_empty(), "unexpected renames: {out:?}");
    }

    #[test]
    fn ctor_equals_type_name_is_renamed() {
        let out = compute_ctor_renames(
            &set(&["U", "U_ctor"]),
            &[("W".to_owned(), vec!["U".to_owned()])],
        );
        // `U_ctor` taken -> falls through to `U_ctor2`.
        assert_eq!(out.get("U"), Some(&"U_ctor2".to_owned()));
    }

    #[test]
    fn multi_owner_collision_is_renamed() {
        let out = compute_ctor_renames(
            &set(&["A", "B"]),
            &[
                ("A".to_owned(), vec!["Some".to_owned()]),
                ("B".to_owned(), vec!["Some".to_owned()]),
            ],
        );
        assert_eq!(out.get("Some"), Some(&"Some_ctor".to_owned()));
    }

    #[test]
    fn deterministic_and_pure() {
        let types = set(&["T", "X"]);
        let unions = vec![
            ("U1".to_owned(), vec!["X".to_owned(), "Y".to_owned()]),
            ("U2".to_owned(), vec!["Y".to_owned(), "Z".to_owned()]),
        ];
        let a = compute_ctor_renames(&types, &unions);
        let b = compute_ctor_renames(&types, &unions);
        assert_eq!(a, b);
        assert_eq!(a.get("X"), Some(&"X_ctor".to_owned()));
        assert_eq!(a.get("Y"), Some(&"Y_ctor".to_owned()));
        assert!(!a.contains_key("Z"));
    }
}

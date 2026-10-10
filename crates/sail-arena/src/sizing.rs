//! Shard-groessenabhaengiges Sizing: Reservierung pro Shard-Groesse.
//!
//! Die Policy ist absichtlich nicht-generisch (bloat-arm): reine
//! `usize`-Arithmetik, keine Arena-Logik. Sie entscheidet nur,
//! wie viele Teil-Arenen mit welchem Live-Budget angelegt werden.
//!
//! Hintergrund: [`crate::shard::ShardArena`] verteilt Eintraege so,
//! dass kein Shard mehr als `per_shard_budget` lebende Eintraege haelt.
//! Das Budget folgt der erwarteten Gesamtgroesse ([`ShardSize`]), damit
//! kleine Modelle (Lane 14: `aarch64_small`) keine ueberdimensionierten
//! Arenen reservieren und grosse Modelle frueh in drop-bare Shards fallen.

/// Shard-Groessenklasse, abgeleitet aus der erwarteten Eintragszahl.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShardSize {
    /// Bis [`ShardSize::SMALL_MAX`] Eintraege (z. B. `aarch64_small`-Shards).
    Small,
    /// Bis [`ShardSize::MEDIUM_MAX`] Eintraege.
    Medium,
    /// Alles darueber (z. B. `arm-v9.4-a`-Shards).
    Large,
}

impl ShardSize {
    /// Obergrenze der Klasse [`ShardSize::Small`].
    pub const SMALL_MAX: usize = 4_096;
    /// Obergrenze der Klasse [`ShardSize::Medium`].
    pub const MEDIUM_MAX: usize = 65_536;

    /// Live-Budget (Reservierung) pro Shard dieser Klasse.
    ///
    /// Klein genug, dass ein einzelner Shard-Drop spuerbar Peak senkt,
    /// gross genug, dass die Shard-Anzahl handhabbar bleibt.
    #[must_use]
    pub const fn per_shard_budget(self) -> usize {
        match self {
            ShardSize::Small => 1_024,
            ShardSize::Medium => 8_192,
            ShardSize::Large => 65_536,
        }
    }

    /// Klasse aus der erwarteten Gesamteintragszahl ableiten.
    #[must_use]
    pub const fn from_expected_total(expected: usize) -> Self {
        if expected <= Self::SMALL_MAX {
            ShardSize::Small
        } else if expected <= Self::MEDIUM_MAX {
            ShardSize::Medium
        } else {
            ShardSize::Large
        }
    }
}

/// Reservierungsplan: so viele Shards mit je so viel Budget anlegen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShardPlan {
    /// Gewaehlte Groessenklasse.
    pub size: ShardSize,
    /// Live-Budget pro Shard (== `size.per_shard_budget()`).
    pub per_shard_budget: usize,
    /// Anzahl anzulegender Shards; deckt `expected_total` ab.
    pub num_shards: usize,
    /// Erwartete Gesamteintragszahl (Planungsgrundlage).
    pub expected_total: usize,
}

impl ShardPlan {
    /// Gesamtkapazitaet des Plans (`num_shards * per_shard_budget`).
    #[must_use]
    pub fn total_capacity(self) -> usize {
        self.num_shards.saturating_mul(self.per_shard_budget)
    }

    /// `true`, wenn der Plan `expected_total` Eintraege aufnehmen kann.
    #[must_use]
    pub fn covers(self) -> bool {
        self.total_capacity() >= self.expected_total
    }
}

/// Plan fuer `expected_total` Eintraege berechnen.
///
/// Garantien (siehe Policy-Tests): `plan.covers()` ist immer `true`,
/// `num_shards >= 1`, und das Budget stammt aus der passenden
/// [`ShardSize`]-Klasse. Fuer `expected_total == 0` gibt es genau
/// einen leeren Small-Shard.
#[must_use]
pub fn plan_for(expected_total: usize) -> ShardPlan {
    let size = ShardSize::from_expected_total(expected_total);
    let per_shard_budget = size.per_shard_budget();
    // Aufrunden: (n + d - 1) / d, mindestens 1 Shard.
    let num_shards = (expected_total.saturating_add(per_shard_budget - 1)
        / per_shard_budget)
        .max(1);
    ShardPlan {
        size,
        per_shard_budget,
        num_shards,
        expected_total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budgets_are_ordered() {
        assert!(
            ShardSize::Small.per_shard_budget()
                < ShardSize::Medium.per_shard_budget()
        );
        assert!(
            ShardSize::Medium.per_shard_budget()
                < ShardSize::Large.per_shard_budget()
        );
    }

    #[test]
    fn class_boundaries() {
        assert_eq!(ShardSize::from_expected_total(0), ShardSize::Small);
        assert_eq!(
            ShardSize::from_expected_total(ShardSize::SMALL_MAX),
            ShardSize::Small
        );
        assert_eq!(
            ShardSize::from_expected_total(ShardSize::SMALL_MAX + 1),
            ShardSize::Medium
        );
        assert_eq!(
            ShardSize::from_expected_total(ShardSize::MEDIUM_MAX),
            ShardSize::Medium
        );
        assert_eq!(
            ShardSize::from_expected_total(ShardSize::MEDIUM_MAX + 1),
            ShardSize::Large
        );
    }

    #[test]
    fn plan_always_covers() {
        for total in [0, 1, 1_023, 1_024, 1_025, 4_096, 4_097, 65_536, 65_537, 1_000_000] {
            let plan = plan_for(total);
            assert!(plan.covers(), "plan {plan:?} deckt {total} nicht ab");
            assert!(plan.num_shards >= 1);
            assert_eq!(plan.per_shard_budget, plan.size.per_shard_budget());
        }
    }

    #[test]
    fn plan_is_tight() {
        // Einen Shard weniger duerfte nicht mehr reichen (ausser total == 0).
        for total in [1, 1_024, 1_025, 100_000] {
            let plan = plan_for(total);
            let smaller = (plan.num_shards - 1).saturating_mul(plan.per_shard_budget);
            assert!(smaller < total, "plan {plan:?} ist nicht knapp");
        }
    }
}

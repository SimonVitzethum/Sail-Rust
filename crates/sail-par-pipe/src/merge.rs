//! Deterministischer Merge der Shard-Ergebnisse (Skizze).
//!
//! Alle Pipeline-Ausgaben tragen ihren Eingangsindex (`order`); der Merge
//! sortiert danach und macht das Ergebnis unabhängig vom Thread-Scheduling
//! bit-identisch. Die `rayon`-Variante nutzt den globalen Pool für große
//! Shard-Zahlen.

use crate::pipeline::{ShardOutput, StageError};

/// Sortiert `(Eingangsindex, Wert)`-Paare in Eingangsreihenfolge und
/// streift die Indizes ab. Stabil: gleiche Indizes behalten ihre relative
/// Ordnung (Indizes sind in der Pipeline ohnehin eindeutig).
#[must_use]
pub fn merge_ordered<T>(mut items: Vec<(usize, T)>) -> Vec<T> {
    items.sort_by_key(|(order, _)| *order);
    items.into_iter().map(|(_, value)| value).collect()
}

/// Wie [`merge_ordered`], sortiert aber im globalen rayon-Pool
/// (paralleles Sortieren ab großen Shard-Zahlen).
#[must_use]
pub fn merge_ordered_par<T: Send>(mut items: Vec<(usize, T)>) -> Vec<T> {
    use rayon::prelude::*;
    items.par_sort_by_key(|(order, _)| *order);
    items.into_iter().map(|(_, value)| value).collect()
}

/// Manifest der Shard-IDs in Ausgabe-Reihenfolge (für Diff-Replays und
/// Lane-13-Berichte: Reihenfolge ist Teil des Vertrags).
#[must_use]
pub fn manifest<E>(outputs: &[ShardOutput<E>]) -> Vec<String> {
    outputs.iter().map(|o| o.shard.clone()).collect()
}

/// Konkateniert erfolgreiche Shard-Ausgaben in Merge-Reihenfolge.
///
/// `separator` trennt Shard-Texte (z. B. `"\n"` für Lean-Zeilen).
/// Bei beliebig vielen Shard-Fehlern: `Err` mit **allen** [`StageError`]s
/// (kein stilles Überspringen, kein Abbruch beim ersten Fehler).
pub fn concat_ok<E: AsRef<str>>(
    outputs: &[ShardOutput<E>],
    separator: &str,
) -> Result<String, Vec<StageError>> {
    let errors: Vec<StageError> = outputs
        .iter()
        .filter_map(|o| o.output.as_ref().err().cloned())
        .collect();
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut out = String::new();
    for (i, o) in outputs.iter().enumerate() {
        if i > 0 {
            out.push_str(separator);
        }
        if let Ok(text) = &o.output {
            out.push_str(text.as_ref());
        }
    }
    Ok(out)
}

// Tests als Code (E3: NICHT ausführen; Verifikation via Test-Rig, später).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{ShardOutput, Stage};

    fn ok(order: usize, shard: &str, text: &str) -> ShardOutput<String> {
        ShardOutput {
            order,
            shard: shard.to_string(),
            output: Ok(text.to_string()),
        }
    }

    #[test]
    fn merge_sorts_by_index() {
        let out = merge_ordered(vec![(2, "c"), (0, "a"), (1, "b")]);
        assert_eq!(out, vec!["a", "b", "c"]);
    }

    #[test]
    fn merge_empty_stays_empty() {
        let out: Vec<&str> = merge_ordered(Vec::new());
        assert!(out.is_empty());
    }

    #[test]
    fn par_merge_matches_serial() {
        let items = vec![(3, 30), (1, 10), (2, 20), (0, 0)];
        assert_eq!(merge_ordered_par(items.clone()), merge_ordered(items));
    }

    #[test]
    fn concat_ok_joins_in_order() {
        let outputs = vec![ok(0, "a", "x"), ok(1, "b", "y")];
        assert_eq!(concat_ok(&outputs, "\n").expect("ok"), "x\ny");
    }

    #[test]
    fn concat_ok_collects_all_errors() {
        let outputs = vec![
            ok(0, "a", "x"),
            ShardOutput {
                order: 1,
                shard: "b".to_string(),
                output: Err(StageError::new(
                    Stage::Check,
                    "b".to_string(),
                    "typfehler".to_string(),
                )),
            },
            ShardOutput {
                order: 2,
                shard: "c".to_string(),
                output: Err(StageError::new(
                    Stage::Emit,
                    "c".to_string(),
                    "nicht abbildbar".to_string(),
                )),
            },
        ];
        let errs = concat_ok(&outputs, "\n").expect_err("muss Fehler liefern");
        assert_eq!(errs.len(), 2);
        assert_eq!(errs[0].stage, Stage::Check);
        assert_eq!(errs[1].stage, Stage::Emit);
    }

    #[test]
    fn manifest_lists_shards_in_order() {
        let outputs = vec![ok(0, "instrs32", "x"), ok(1, "sve", "y")];
        assert_eq!(manifest(&outputs), vec!["instrs32", "sve"]);
    }
}

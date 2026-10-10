#![forbid(unsafe_code)]
//! Lane-127 Shard-Begriffsklaerung (Code, E3: NICHT ausfuehren).
//!
//! `sail_rewrite::Shard` ist die Rewrite-Einheit (Def-Chunk, `id: usize`),
//! `sail_shard::Shard` der Datei-Shard (stabile `ShardId`, Zeilen,
//! eigener Inhalt). Verschiedene Begriffe, kein Typ-Duplikat; die
//! Umbenennung (`RewriteUnit`) ist Owner-Folgearbeit (Lane 07).
//! Dieser Test pinnt nur das lokale Chunk-Verhalten (deterministisch,
//! grenzen-treu), ohne neuen Dep.

use sail_rewrite::{Def, DefKind, Shard};

fn def(name: &str) -> Def {
    Def::new(name, DefKind::Function, Vec::new())
}

#[test]
fn chunk_is_deterministic_and_covers_all_defs() {
    let defs: Vec<Def> = (0..7).map(|i| def(&format!("f{i}"))).collect();
    let shards = Shard::chunk(defs.clone(), 3);
    assert_eq!(shards.len(), 3);
    assert_eq!([shards[0].len(), shards[1].len(), shards[2].len()], [3, 3, 1]);
    // IDs sind stabil (Reassembly-Reihenfolge), Inhalte lueckenlos.
    let names: Vec<String> = shards.iter().flat_map(|s| s.defs.iter().map(|d| d.name.clone())).collect();
    let want: Vec<String> = defs.iter().map(|d| d.name.clone()).collect();
    assert_eq!(names, want);
    assert_eq!([shards[0].id, shards[1].id, shards[2].id], [0, 1, 2]);
}

#[test]
fn chunk_size_zero_clamps_to_one_and_empty_stays_empty() {
    let one = Shard::chunk(vec![def("f")], 0);
    assert_eq!(one.len(), 1);
    let empty: Vec<Shard> = Shard::chunk(Vec::new(), 4);
    assert!(empty.is_empty());
    assert!(Shard::new(9, Vec::new()).is_empty());
}

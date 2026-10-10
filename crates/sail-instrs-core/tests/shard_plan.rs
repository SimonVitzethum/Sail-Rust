//! Fixture-Test: Shard-Plan + Dispatch über Mini-Abbilder der Kern-Dateien.
//!
//! Die Fixtures (`tests/fixtures/*.sail`) bilden die realen Muster aus
//! `instrs32.sail` (execute/decode-Paare + `__Decode`-Clauses mit
//! `SEE`-Wächtern) und `v8_base.sail` (Helper-Defs) im Kleinen ab.
#![forbid(unsafe_code)]

use std::fs::File;
use std::io::BufReader;

use sail_instrs_core::dispatch::DispatchGraph;
use sail_instrs_core::measure::{decode_fanout, largest};
use sail_instrs_core::plan::{FilePlan, Shard, shard_of};
use sail_instrs_core::scanner::{DefKind, scan_headers};

fn fixture(name: &str) -> BufReader<File> {
    let path = format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name);
    BufReader::new(File::open(&path).unwrap_or_else(|e| panic!("{path}: {e}")))
}

#[test]
fn instrs_fixture_has_expected_def_kinds() {
    let hs = scan_headers(fixture("mini_instrs.sail")).unwrap();
    let count = |k: DefKind| hs.iter().filter(|h| h.kind == k).count();
    // 2 execute + 3 decode, jeweils mit val-Signatur.
    assert_eq!(count(DefKind::Val), 5);
    assert_eq!(count(DefKind::Function), 5);
    assert_eq!(count(DefKind::Clause), 3);
}

#[test]
fn instrs_fixture_shard_plan() {
    let hs = scan_headers(fixture("mini_instrs.sail")).unwrap();
    let mut plan = FilePlan::default();
    for h in &hs {
        plan.add(h);
    }
    // val+function je execute (2x2) und decode (3x2); 3 dispatch-Clauses.
    assert_eq!(plan.exec, 4);
    assert_eq!(plan.decode, 6);
    assert_eq!(plan.dispatch, 3);
    assert_eq!(plan.helper, 0);
    assert_eq!(plan.total(), hs.len() as u64);
    // Clause-Ziele sind Dispatch, egal wie sie heißen.
    for h in hs.iter().filter(|h| h.kind == DefKind::Clause) {
        assert_eq!(shard_of(h), Shard::Dispatch);
    }
}

#[test]
fn instrs_fixture_decode_execute_is_one_to_one() {
    let g = DispatchGraph::build(fixture("mini_instrs.sail")).unwrap();
    // 3 decode-Funktionen, jede mit genau einer execute-Kante.
    assert_eq!(g.decode_edges.len(), 3);
    assert!(g.decode_without_single_execute().is_empty());
    // execute wird über Enkodierungen geteilt (ADC: A1 + T1 -> 1 execute).
    let adc: Vec<_> = g
        .decode_edges
        .iter()
        .filter(|e| e.execute == "execute_aarch32_instrs_ADC_i_Op_A_txt")
        .collect();
    assert_eq!(adc.len(), 2);
}

#[test]
fn instrs_fixture_clause_dispatch_with_see_order() {
    let g = DispatchGraph::build(fixture("mini_instrs.sail")).unwrap();
    assert_eq!(g.clause_edges.len(), 3);
    assert!(g.see_monotonic_violations().is_empty());
    let t32: Vec<_> = g.clause_edges.iter().filter(|e| e.target == "__DecodeT32").collect();
    assert_eq!(t32.len(), 2);
    assert_eq!(t32[0].see_guard, Some(2814));
    assert_eq!(t32[1].see_guard, Some(2815));
}

#[test]
fn instrs_fixture_largest_and_fanout() {
    let top = largest(fixture("mini_instrs.sail"), 2).unwrap();
    assert_eq!(top.len(), 2);
    // ADC-execute hat die meisten Zeilen, VMOV die meisten Arme-Anteile.
    assert_eq!(top[0].name, "execute_aarch32_instrs_ADC_i_Op_A_txt");
    assert!(top[0].is_execute);
    let fan = decode_fanout(fixture("mini_instrs.sail")).unwrap();
    assert_eq!(fan[0], ("__DecodeT32".to_string(), 2));
    assert_eq!(fan[1], ("__DecodeA32_DataProMisc".to_string(), 1));
}

#[test]
fn v8base_fixture_is_all_helper() {
    let hs = scan_headers(fixture("mini_v8base.sail")).unwrap();
    assert!(!hs.is_empty());
    for h in &hs {
        assert_eq!(shard_of(h), Shard::Helper, "unexpected shard for {}", h.name);
    }
    let kinds: Vec<DefKind> = hs.iter().map(|h| h.kind).collect();
    assert!(kinds.contains(&DefKind::Other)); // enum/bitfield/register/type/let/overload
    let top = largest(fixture("mini_v8base.sail"), 1).unwrap();
    assert_eq!(top[0].name, "FPExpCoefficient_read");
    assert_eq!(top[0].arms, 4);
}

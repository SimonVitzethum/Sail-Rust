//! Vermessung: größte Funktionen je Datei (Zeilen, Match-Arme, `if`s).
//!
//! Jede Sail-`function` wird genau eine Lean-Funktion; gestreute
//! `__Decode…`-Funktionen werden je eine Lean-Funktion mit einem Arm pro
//! Clause (siehe [`crate::dispatch`]). [`largest`] meldet die Top-N
//! Funktionsrümpfe, [`decode_fanout`] die Clause-Zahl je Dispatch-Ziel —
//! beides im Streaming-Verfahren ohne die Datei zu materialisieren.

use std::io::BufRead;

use crate::scanner::{self, DefKind};

/// Vermessung eines einzelnen Funktionsrumpfs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuncStat {
    pub name: String,
    pub line: u64,
    pub lines: u64,
    pub arms: u64,
    pub ifs: u64,
    pub is_execute: bool,
    pub is_decode: bool,
}

/// Die `n` größten `function`-Rümpfe (nach Zeilen) aus einem `.sail`-Strom.
///
/// Nur `DefKind::Function` zählt; `val`-Signaturen und Clauses bleiben außen
/// vor. Speicher: O(n) Treffer, nicht O(Datei).
pub fn largest(reader: impl BufRead, n: usize) -> std::io::Result<Vec<FuncStat>> {
    let mut top: Vec<FuncStat> = Vec::new();
    scanner::fold_bodies(reader, |h, s| {
        if h.kind != DefKind::Function {
            return;
        }
        let stat = FuncStat {
            name: h.name.clone(),
            line: h.line,
            lines: s.lines,
            arms: s.arms,
            ifs: s.ifs,
            is_execute: h.name.starts_with("execute_"),
            is_decode: h.name.starts_with("decode_"),
        };
        top.push(stat);
        if top.len() > n * 2 {
            top.sort_by_key(|b| std::cmp::Reverse(b.lines));
            top.truncate(n);
        }
    })?;
    top.sort_by_key(|b| std::cmp::Reverse(b.lines));
    top.truncate(n);
    Ok(top)
}

/// Clause-Zahl je `__Decode…`-Ziel (Arm-Zahl der künftigen Lean-Funktion).
///
/// Sortiert absteigend nach Clause-Zahl.
pub fn decode_fanout(reader: impl BufRead) -> std::io::Result<Vec<(String, u64)>> {
    use std::collections::BTreeMap;
    let headers = scanner::scan_headers(reader)?;
    let mut counts: BTreeMap<&str, u64> = BTreeMap::new();
    for h in &headers {
        if h.kind == DefKind::Clause {
            *counts.entry(h.name.as_str()).or_insert(0) += 1;
        }
    }
    let mut out: Vec<(String, u64)> =
        counts.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    out.sort_by_key(|b| std::cmp::Reverse(b.1));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    const MINI: &str = "function execute_big (a) = {\n\
        match a {\n\
        0b00 => 1,\n\
        0b01 => 2,\n\
        0b10 => 3,\n\
        _ => 4,\n\
        }\n\
        }\n\
        \n\
        function execute_small (a) = {\n\
        a\n\
        }\n\
        \n\
        function clause __DecodeT32 ((pc, o)) if SEE < 1) = {\n\
        decode_a(o)\n\
        }\n\
        \n\
        function clause __DecodeT32 ((pc, o)) if SEE < 2) = {\n\
        decode_b(o)\n\
        }\n";

    #[test]
    fn largest_sorts_and_skips_clauses() {
        let top = largest(BufReader::new(MINI.as_bytes()), 5).unwrap();
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].name, "execute_big");
        assert_eq!(top[0].arms, 4);
        assert!(top[0].is_execute);
        assert_eq!(top[1].name, "execute_small");
    }

    #[test]
    fn fanout_counts_clauses_per_target() {
        let fan = decode_fanout(BufReader::new(MINI.as_bytes())).unwrap();
        assert_eq!(fan, vec![("__DecodeT32".to_string(), 2)]);
    }
}

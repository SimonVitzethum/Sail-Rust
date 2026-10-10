//! Decode-Dispatch: Kanten `Clause → decode_* → execute_*`.
//!
//! Struktur im ARMv9-Modell (verifiziert an `instrs32/64.sail`):
//!
//! - Jede `decode_*`-Funktion ruft **genau eine** `execute_*`-Funktion als
//!   Tail-Call (ggf. hinter `ConditionPassed()`); `execute_*` wird über
//!   Enkodierungen hinweg geteilt (n:1).
//! - Jede `function clause __Decode…` extrahiert per `Slice` Opcode-Felder,
//!   ruft **genau eine** `decode_*`-Funktion und trägt einen
//!   `SEE < N`-Prioritätswächter (Erste-passende-Regel, kein Sprungtableau).
//! - Die gestreuten `__Decode…`-Funktionen (Dekl. in `decode_start.sail`,
//!   Defaults in `decode_end.sail`) werden in Lean je **eine** Funktion mit
//!   einem Match-Arm pro Clause.

use std::io::BufRead;

use crate::scanner::{self, DefKind};

/// Kante `decode_* → execute_*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeEdge {
    pub decode: String,
    pub execute: String,
}

/// Kante `Clause (Dispatch-Ziel) → decode_*` mit Prioritätswächter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClauseEdge {
    pub target: String,
    pub decode: String,
    /// `SEE < N` aus der Clause-Kopfzeile, falls vorhanden.
    pub see_guard: Option<u64>,
    pub line: u64,
}

/// Extrahiert beide Kantenarten in einem Streaming-Durchlauf.
#[derive(Debug, Default)]
pub struct DispatchGraph {
    pub decode_edges: Vec<DecodeEdge>,
    pub clause_edges: Vec<ClauseEdge>,
}

impl DispatchGraph {
    /// Baut den Graphen aus einem `.sail`-Strom (O(1) Zusatzspeicher pro Def).
    pub fn build(reader: impl BufRead) -> std::io::Result<Self> {
        let mut g = Self::default();
        // Clause-Kopfzeilen für `see_guard`: Kopfzeile merken.
        let mut head_text = String::new();
        let mut head_line_no: u64 = 0;
        let mut cur: Option<(DefKind, String, u64)> = None;
        let mut calls_exec: Vec<String> = Vec::new();
        let mut calls_dec: Vec<String> = Vec::new();

        // Wir lesen zeilenweise selbst, weil `see_guard` die Kopfzeile braucht.
        let mut raw = String::new();
        let mut reader = reader;
        let mut lineno: u64 = 0;
        loop {
            raw.clear();
            if reader.read_line(&mut raw)? == 0 {
                break;
            }
            lineno += 1;
            let line = raw.trim_end_matches(['\r', '\n']);
            if let Some((kind, name)) = scanner::classify_head(line) {
                if let Some((ck, cn, _)) = cur.take() {
                    g.flush(&ck, &cn, &calls_exec, &calls_dec, &head_text, head_line_no);
                    calls_exec.clear();
                    calls_dec.clear();
                }
                head_text = line.to_string();
                head_line_no = lineno;
                cur = Some((kind, name, lineno));
            } else if cur.is_some() {
                scanner::calls_in_line(line, &mut calls_exec, &mut calls_dec);
            }
        }
        if let Some((ck, cn, _)) = cur.take() {
            g.flush(&ck, &cn, &calls_exec, &calls_dec, &head_text, head_line_no);
        }
        Ok(g)
    }

    fn flush(
        &mut self,
        kind: &DefKind,
        name: &str,
        calls_exec: &[String],
        calls_dec: &[String],
        head_text: &str,
        head_line: u64,
    ) {
        if *kind == DefKind::Function && name.starts_with("decode_") {
            for e in calls_exec {
                self.decode_edges.push(DecodeEdge {
                    decode: name.to_string(),
                    execute: e.clone(),
                });
            }
        }
        if *kind == DefKind::Clause {
            for d in calls_dec {
                self.clause_edges.push(ClauseEdge {
                    target: name.to_string(),
                    decode: d.clone(),
                    see_guard: scanner::see_guard(head_text),
                    line: head_line,
                });
            }
        }
    }

    /// `decode_*`-Namen ohne genau eine `execute_*`-Kante (sollte leer sein).
    pub fn decode_without_single_execute(&self) -> Vec<&str> {
        use std::collections::BTreeMap;
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        let mut seen: Vec<&str> = Vec::new();
        for e in &self.decode_edges {
            *counts.entry(e.decode.as_str()).or_insert(0) += 1;
            if !seen.contains(&e.decode.as_str()) {
                seen.push(e.decode.as_str());
            }
        }
        // Hinweis: `seen` allein genügt nicht — decode-Funktionen ganz ohne
        // Kante erscheinen in keiner Edge. Diese Prüfung deckt der Fixture-
        // Test über die Header-Liste ab; hier zählen wir Mehrfach-Kanten.
        counts
            .into_iter()
            .filter(|(_, c)| *c != 1)
            .map(|(d, _)| d)
            .collect()
    }

    /// Prüft, dass `SEE`-Wächter je Dispatch-Ziel streng monoton steigen
    /// (Dateireihenfolge = Prioritätsreihenfolge).
    pub fn see_monotonic_violations(&self) -> Vec<(&str, u64)> {
        use std::collections::BTreeMap;
        let mut last: BTreeMap<&str, u64> = BTreeMap::new();
        let mut out = Vec::new();
        for e in &self.clause_edges {
            if let Some(n) = e.see_guard {
                if let Some(prev) = last.get(e.target.as_str()) {
                    if n <= *prev {
                        out.push((e.target.as_str(), e.line));
                    }
                }
                last.insert(e.target.as_str(), n);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    const MINI: &str = "function decode_a (x) = {\n\
        execute_shared(x)\n\
        }\n\
        \n\
        function decode_b (x) = {\n\
        execute_shared(x)\n\
        }\n\
        \n\
        function clause __DecodeT32 ((pc, o)) if SEE < 3) = {\n\
        decode_a(o)\n\
        }\n\
        \n\
        function clause __DecodeT32 ((pc, o)) if SEE < 9) = {\n\
        decode_b(o)\n\
        }\n";

    #[test]
    fn edges_extracted() {
        let g = DispatchGraph::build(BufReader::new(MINI.as_bytes())).unwrap();
        assert_eq!(g.decode_edges.len(), 2);
        assert!(g.decode_without_single_execute().is_empty());
        assert_eq!(g.clause_edges.len(), 2);
        assert_eq!(g.clause_edges[0].see_guard, Some(3));
        assert!(g.see_monotonic_violations().is_empty());
    }

    #[test]
    fn non_monotonic_see_detected() {
        let bad = MINI.replace("SEE < 9", "SEE < 2");
        let g = DispatchGraph::build(BufReader::new(bad.as_bytes())).unwrap();
        assert_eq!(g.see_monotonic_violations().len(), 1);
    }
}

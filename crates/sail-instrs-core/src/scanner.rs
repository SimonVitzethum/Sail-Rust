//! Streaming-Erkennung von Sail-Top-Level-Definitionen.
//!
//! Der Scanner liest eine `.sail`-Datei als [`BufRead`]-Strom und meldet nur
//! Kopfzeilen (`val`, `function`, `function clause`, `scattered function`,
//! `mapping`, `type`, `enum`, `register`, `let`, `overload`, `bitfield`).
//! Rümpfe werden nie zwischengespeichert: [`fold_bodies`] faltet sie direkt in
//! eine vom Aufrufer gehaltene Statistik.

use std::io::{BufRead, Result};

/// Art einer Top-Level-Definition (nur Kopfzeilen-Keywords).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefKind {
    /// `val name : ...` — Typsignatur, gehört zum selben Shard wie die Impl.
    Val,
    /// `function name ...` — Funktionsrumpf.
    Function,
    /// `function clause Target ...` — ein Dispatch-Arm einer gestreuten Funktion.
    Clause,
    /// `scattered function ...` — Deklaration (liegt in `decode_start.sail`).
    Scattered,
    /// Alles andere auf Top-Level (`type`, `enum`, `register`, `let`, …).
    Other,
}

/// Kopf einer Top-Level-Definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefHeader {
    pub kind: DefKind,
    pub name: String,
    /// 1-basierte Zeilennummer der Kopfzeile.
    pub line: u64,
}

/// Body-Statistik einer einzelnen Funktionsdefinition.
///
/// Wird pro Definition frisch aufgebaut und danach verworfen (low-RAM).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BodyStats {
    /// Zeilen ab Kopfzeile bis (exklusiv) zur nächsten Kopfzeile.
    pub lines: u64,
    /// Vorkommen von `=>` (Match-Arme; Proxy für Lean-Match-Arme).
    pub arms: u64,
    /// Vorkommen des Tokens `if` (Verzweigungen).
    pub ifs: u64,
    /// Aufgerufene `execute_*`-Funktionen (nur Calls mit `(`).
    pub calls_execute: Vec<String>,
    /// Aufgerufene `decode_*`-Funktionen (nur Calls mit `(`).
    pub calls_decode: Vec<String>,
}

/// Erkennt, ob `line` eine Top-Level-Kopfzeile ist (Spalte 0, kein Einzug).
///
/// Gibt `(DefKind, Name)` zurück. `function clause` wird vor `function`
/// geprüft; der Name einer Clause ist das Dispatch-Ziel (`__Decode…`).
pub fn classify_head(line: &str) -> Option<(DefKind, String)> {
    if line.starts_with(' ') || line.starts_with('\t') {
        return None;
    }
    // Längste Präfixe zuerst.
    const PREFIXES: &[(&str, DefKind)] = &[
        ("function clause ", DefKind::Clause),
        ("scattered function ", DefKind::Scattered),
        ("function ", DefKind::Function),
        ("val ", DefKind::Val),
    ];
    for (prefix, kind) in PREFIXES {
        if let Some(rest) = line.strip_prefix(prefix) {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '\'')
                .collect();
            if name.is_empty() {
                return None;
            }
            return Some((*kind, name));
        }
    }
    // Übrige Top-Level-Keywords zählen als Shard-Grenze (Other).
    const OTHER: &[&str] = &[
        "mapping ",
        "scattered ",
        "type ",
        "enum ",
        "register ",
        "let ",
        "overload ",
        "bitfield ",
        "union ",
    ];
    for prefix in OTHER {
        if let Some(rest) = line.strip_prefix(prefix) {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '\'')
                .collect();
            return Some((DefKind::Other, name));
        }
    }
    None
}

/// Sammelt nur Kopfzeilen (Name + Art + Zeile); Rümpfe werden übersprungen.
///
/// Speicher: O(Anzahl Definitionen), nicht O(Dateigröße).
pub fn scan_headers(reader: impl BufRead) -> Result<Vec<DefHeader>> {
    let mut out = Vec::new();
    fold_bodies(reader, |header, _| {
        out.push(header.clone());
    })?;
    Ok(out)
}

/// Faltet jede Definition (Kopf + Rumpf-Statistik) durch `f`.
///
/// Es wird immer nur der aktuelle Rumpf gehalten; danach wird er verworfen.
pub fn fold_bodies(mut reader: impl BufRead, mut f: impl FnMut(&DefHeader, &BodyStats)) -> Result<()> {
    let mut line = String::new();
    let mut lineno: u64 = 0;
    let mut cur: Option<DefHeader> = None;
    let mut stats = BodyStats::default();

    let flush = |cur: &mut Option<DefHeader>, stats: &mut BodyStats, f: &mut dyn FnMut(&DefHeader, &BodyStats)| {
        if let Some(h) = cur.take() {
            f(&h, stats);
            *stats = BodyStats::default();
        }
    };

    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        lineno += 1;
        // `read_line` behält `\n`; für die Klassifikation stört das nicht.
        let trimmed_end = line.trim_end_matches(['\r', '\n']);
        if let Some((kind, name)) = classify_head(trimmed_end) {
            flush(&mut cur, &mut stats, &mut f);
            cur = Some(DefHeader { kind, name, line: lineno });
            stats.lines = 1;
        } else if cur.is_some() {
            stats.lines += 1;
            stats.arms += trimmed_end.matches("=>").count() as u64;
            stats.ifs += count_if_tokens(trimmed_end) as u64;
            calls_in_line(trimmed_end, &mut stats.calls_execute, &mut stats.calls_decode);
        }
    }
    flush(&mut cur, &mut stats, &mut f);
    Ok(())
}

/// Zählt `if`-Tokens (keine Substrings wie `notify`/`sift`).
fn count_if_tokens(line: &str) -> usize {
    line.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| *t == "if")
        .count()
}

/// Sammelt `execute_*(`- und `decode_*(`-Aufrufe aus einer Rumpfzeile.
///
/// Erkannt wird der Bezeichner, gefolgt von optionalem Leerraum und `(` —
/// Fließtext ohne Klammer (z. B. in Kommentaren) zählt nicht.
/// Jeder Name wird höchstens einmal je übergebener Liste aufgenommen.
pub fn calls_in_line(line: &str, exec: &mut Vec<String>, dec: &mut Vec<String>) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &line[i..];
        let is_exec = if rest.starts_with("execute_") {
            true
        } else if rest.starts_with("decode_") {
            false
        } else {
            i += 1;
            continue;
        };
        // Wortgrenze davor: kein Bezeichner-Zeichen unmittelbar links.
        if i > 0 {
            let prev = bytes[i - 1] as char;
            if prev.is_alphanumeric() || prev == '_' {
                i += 1;
                continue;
            }
        }
        let mut j = i;
        while j < bytes.len() {
            let c = bytes[j] as char;
            if c.is_alphanumeric() || c == '_' || c == '\'' {
                j += 1;
            } else {
                break;
            }
        }
        let name = &line[i..j];
        // Name muss über den reinen Präfix (`execute_`/`decode_`, je 7
        // Zeichen) hinausgehen; der Rest ist ein echter Bezeichner.
        if name.len() > 7 && line[j..].trim_start().starts_with('(') {
            let list = if is_exec { &mut *exec } else { &mut *dec };
            if !list.iter().any(|s| s == name) {
                list.push(name.to_string());
            }
        }
        i = j.max(i + 1);
    }
}

/// Extrahiert den `SEE < N`-Wächter aus einer Clause-Kopfzeile.
pub fn see_guard(head_line: &str) -> Option<u64> {
    let idx = head_line.find("SEE <")?;
    head_line[idx + 5..]
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse::<u64>()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    const MINI: &str = "val execute_x : (bits(4)) -> unit\n\
        \n\
        function execute_x (a) = {\n\
            match a {\n\
            0b00 => 1,\n\
            _ => 2,\n\
            }\n\
        }\n\
        \n\
        function clause __DecodeT32 ((pc, opcode)) if SEE < 7) = {\n\
            execute_x(opcode)\n\
        }\n";

    #[test]
    fn heads_found_in_order() {
        let hs = scan_headers(BufReader::new(MINI.as_bytes())).unwrap();
        assert_eq!(hs.len(), 3);
        assert_eq!(hs[0].kind, DefKind::Val);
        assert_eq!(hs[0].name, "execute_x");
        assert_eq!(hs[0].line, 1);
        assert_eq!(hs[1].kind, DefKind::Function);
        assert_eq!(hs[2].kind, DefKind::Clause);
        assert_eq!(hs[2].name, "__DecodeT32");
    }

    #[test]
    fn indented_lines_are_not_heads() {
        assert_eq!(classify_head("  function f (x) = {"), None);
        assert_eq!(classify_head("\tval x : int"), None);
    }

    #[test]
    fn body_stats_count_arms_ifs_calls() {
        let mut seen = Vec::new();
        fold_bodies(BufReader::new(MINI.as_bytes()), |h, s| {
            seen.push((h.name.clone(), s.clone()));
        })
        .unwrap();
        let exec = &seen[1].1;
        assert_eq!(exec.arms, 2);
        let clause = &seen[2].1;
        assert_eq!(clause.calls_execute, vec!["execute_x".to_string()]);
    }

    #[test]
    fn see_guard_parsed() {
        assert_eq!(see_guard("function clause __DecodeT32 ((pc, o)) if SEE < 2814) = {"), Some(2814));
        assert_eq!(see_guard("function clause __DecodeA64(pc, opcode) ="), None);
        assert_eq!(see_guard("... & SEE < 42 & ..."), Some(42));
    }
}

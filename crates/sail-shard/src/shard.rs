//! Shard-Typ: Dateigrenzen aus `file_start`/`file_end`-Pragmas + Extra-Splits.

use std::fmt;

/// Stabile Shard-Identitaet (Dateiname + Chunk-Index).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ShardId {
    /// Basisname, z. B. `instrs32` (ohne Endung, normalisiert).
    pub name: String,
    /// Chunk-Index innerhalb der Datei (0 = erster/ungueteilter Shard).
    pub chunk: u32,
}

impl ShardId {
    /// Erzeugt eine neue Shard-ID.
    #[must_use]
    pub fn new(name: impl Into<String>, chunk: u32) -> Self {
        Self {
            name: normalize_name(&name.into()),
            chunk,
        }
    }
}

impl fmt::Display for ShardId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.chunk == 0 {
            write!(f, "{}", self.name)
        } else {
            write!(f, "{}#{}", self.name, self.chunk)
        }
    }
}

fn normalize_name(raw: &str) -> String {
    // "instrs32.sail" -> "instrs32"; Pfade auf Basisname reduzieren.
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let stem = base.strip_suffix(".sail").unwrap_or(base);
    stem.trim().to_string()
}

/// Herkunft eines Shards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShardKind {
    /// Direkt aus einer `file_start`/`file_end`-Spanne (oder Gesamtinput).
    File,
    /// Extra-Split-Chunk einer grossen Datei (zeilenbegrenzt).
    SplitChunk,
    /// Kuentslich (z. B. Preamble vor dem ersten Marker).
    Synthetic,
}

/// Ein Shard: atomare Inkrementalitaets-Einheit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shard {
    /// Stabile ID (Name + Chunk).
    pub id: ShardId,
    /// Herkunft.
    pub kind: ShardKind,
    /// 1-basierte Zeilennummern in der Eingabe, aus der der Shard stammt
    /// (bei [`shard_files`] also pro Datei; inklusiv/exklusiv).
    pub start_line: usize,
    /// Exklusive Endzeile (wie `Vec::len`-Semantik auf Zeilen).
    pub end_line: usize,
    /// Inhalt des Shards (eigene Kopie; Shards werden stromweise verarbeitet).
    pub contents: String,
}

impl Shard {
    /// Zeilenzahl des Shards.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.end_line.saturating_sub(self.start_line)
    }
}

/// Extrahiert einen Dateinamen aus einer Marker-Zeile.
///
/// Akzeptiert u. a.:
/// ```text
/// pragma file_start "instrs32.sail"
/// $file_start "v8_base.sail"
/// // file_start "sve.sail"
/// ```
/// Rueckgabe ist der rohe Name (mit Endung); Normalisierung macht [`ShardId`].
fn marker_name(line: &str, marker: &str) -> Option<String> {
    let idx = line.find(marker)?;
    let rest = &line[idx + marker.len()..];
    // Erster und zweiter Anfuehrungsstrich (einfach oder doppelt).
    let rest = rest.trim_start();
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let inner = &rest[quote.len_utf8()..];
    let end = inner.find(quote)?;
    Some(inner[..end].to_string())
}

fn is_start(line: &str) -> Option<String> {
    marker_name(line, "file_start")
}

fn is_end(line: &str) -> bool {
    line.contains("file_end")
}

/// Teilt `source` an `file_start`/`file_end`-Pragmas in Shards.
///
/// Regeln:
/// * Jede `file_start "name"`-Zeile beginnt einen neuen Shard (Name aus dem
///   Marker). Die Marker-Zeile selbst gehoert zum neuen Shard.
/// * `file_end` schliesst den aktuellen Shard ab (Zeile gehoert noch dazu).
/// * Text vor dem ersten Marker wird als synthetischer `preamble`-Shard
///   abgelegt (nur wenn nicht leer, sonst entfällt er); namenloser Text
///   danach (zwischen `file_end` und naechstem `file_start`, oder nach dem
///   letzten `file_end`) wird als durchnummerierter synthetischer
///   `postamble`-Shard (`postamble`, `postamble#1`, ...) abgelegt, damit
///   Shard-IDs stets eindeutig bleiben.
/// * Ohne jeglichen Marker: genau ein Shard `default`.
/// * Unausgeglichene Marker (fehlendes `file_end`) werden tolerant behandelt:
///   Der Shard laeuft bis zum Input-Ende.
#[must_use]
pub fn split_by_file_markers(source: &str) -> Vec<Shard> {
    let lines: Vec<&str> = source.lines().collect();
    if lines.is_empty() {
        return vec![Shard {
            id: ShardId::new("default", 0),
            kind: ShardKind::File,
            start_line: 1,
            end_line: 1,
            contents: String::new(),
        }];
    }

    let mut shards: Vec<Shard> = Vec::new();
    let mut cur_name: Option<String> = None;
    let mut cur_kind = ShardKind::File;
    let mut cur_start: usize = 1; // 1-basiert
    let mut buf: Vec<&str> = Vec::new();
    // Zaehler fuer namenlose Luecken-Shards ("postamble", "postamble#1", ...),
    // damit Shard-IDs auch bei mehreren Luecken eindeutig bleiben.
    let mut gap_count: u32 = 0;

    // Lagert den Puffer als Shard mit fertiger ID ab (leerer Puffer -> kein Shard).
    let flush = |id: ShardId,
                     kind: ShardKind,
                     start: usize,
                     end: usize,
                     buf: &mut Vec<&str>,
                     out: &mut Vec<Shard>| {
        if buf.is_empty() {
            return;
        }
        out.push(Shard {
            id,
            kind,
            start_line: start,
            end_line: end,
            contents: buf.join("\n"),
        });
        buf.clear();
    };
    // ID fuer namenlosen Puffer: fuehrend -> "preamble", sonst durchnummeriert
    // "postamble[#n]".
    let gap_id = |shards_empty: bool, gap_count: &mut u32| -> ShardId {
        if shards_empty {
            ShardId::new("preamble", 0)
        } else {
            let id = ShardId::new("postamble", *gap_count);
            *gap_count += 1;
            id
        }
    };

    for (i, line) in lines.iter().enumerate() {
        let lineno = i + 1; // 1-basiert
        if let Some(name) = is_start(line) {
            // Vorherigen Shard abschliessen: benannt -> sein Name, namenlos ->
            // synthetischer Luecken-Shard.
            if !buf.is_empty() {
                let (id, kind) = match cur_name.take() {
                    Some(n) => (ShardId::new(n, 0), cur_kind),
                    None => (gap_id(shards.is_empty(), &mut gap_count), ShardKind::Synthetic),
                };
                flush(id, kind, cur_start, lineno, &mut buf, &mut shards);
            }
            cur_name = Some(name);
            cur_kind = ShardKind::File;
            cur_start = lineno;
            buf.push(line);
        } else if is_end(line) && cur_name.is_some() {
            buf.push(line);
            let name = cur_name.take().expect("checked is_some");
            flush(
                ShardId::new(name, 0),
                cur_kind,
                cur_start,
                lineno + 1,
                &mut buf,
                &mut shards,
            );
            cur_kind = ShardKind::File;
            cur_start = lineno + 1;
        } else {
            if buf.is_empty() && cur_name.is_none() {
                cur_start = lineno;
            }
            buf.push(line);
        }
    }

    // Rest.
    if !buf.is_empty() {
        let (id, kind) = match cur_name.take() {
            Some(n) => (ShardId::new(n, 0), cur_kind),
            // Ganz ohne Marker: genau ein Shard "default".
            None if shards.is_empty() => (ShardId::new("default", 0), ShardKind::File),
            None => (gap_id(false, &mut gap_count), ShardKind::Synthetic),
        };
        flush(id, kind, cur_start, lines.len() + 1, &mut buf, &mut shards);
    }

    debug_assert!(!shards.is_empty());
    debug_assert_unique_ids(&shards);
    shards
}

/// Prueft (nur Debug), dass Shard-IDs eindeutig sind: doppelte IDs wuerden im
/// [`crate::IncrementalCache`] Fingerprints vermischen.
fn debug_assert_unique_ids(shards: &[Shard]) {
    if cfg!(debug_assertions) {
        let mut seen = std::collections::HashSet::new();
        for s in shards {
            assert!(seen.insert(&s.id), "doppelte Shard-ID: {}", s.id);
        }
    }
}

/// Dateinamen, die pro Instr-Datei Extra-Splits bekommen.
///
/// Deckungsgleich mit der Lane-Spec: `instrs32`/`instrs64`/SVE/SME,
/// `v8_base`, `sysregs_autogen`.
pub const LARGE_INSTR_FILES: &[&str] = &[
    "instrs32",
    "instrs64",
    "sve",
    "sme",
    "v8_base",
    "sysregs_autogen",
];

/// Prueft, ob ein Shard-Name zu einer grossen Datei mit Extra-Splits gehoert.
#[must_use]
pub fn is_large_instr_file(name: &str) -> bool {
    let norm = normalize_name(name);
    LARGE_INSTR_FILES.iter().any(|n| *n == norm)
}

/// Teilt einen Shard in zeilenbegrenzte Chunks (`max_lines` pro Chunk).
///
/// * `max_lines == 0` wird wie `1` behandelt.
/// * Passt der Shard bereits, wird er unveraendert (ein Element) zurueckgegeben.
/// * Chunk-IDs: `<name>#<i>` (`chunk = i`), erster Chunk behaelt `chunk = 0`.
/// * Zeilennummern bleiben auf den Input bezogen, aus dem `shard` stammt
///   (bei [`shard_files`] also pro Datei).
#[must_use]
pub fn extra_split(shard: &Shard, max_lines: usize) -> Vec<Shard> {
    let max_lines = max_lines.max(1);
    let lines: Vec<&str> = shard.contents.lines().collect();
    if lines.len() <= max_lines {
        return vec![shard.clone()];
    }
    lines
        .chunks(max_lines)
        .enumerate()
        .map(|(i, chunk)| {
            let start = shard.start_line + i * max_lines;
            Shard {
                id: ShardId::new(shard.id.name.clone(), i as u32),
                kind: ShardKind::SplitChunk,
                start_line: start,
                end_line: start + chunk.len(),
                contents: chunk.join("\n"),
            }
        })
        .collect()
}

/// Dokumentierte ARMv9-Shardliste in kanonischer Reihenfolge.
///
/// Dient als Referenz/Reihenfolgevorgabe fuer Treiber (Done-Kriterium:
/// "ARMv9-Shardliste dokumentiert").
pub const ARMV9_SHARD_ORDER: &[&str] = &[
    "preamble",
    "v8_base",
    "instrs32",
    "instrs64",
    "sve",
    "sme",
    "sysregs",
    "sysregs_autogen",
    "decode",
    "execute",
    "misc",
];

/// Gibt die kanonische ARMv9-Shardliste als `String`-Vektor zurueck.
#[must_use]
pub fn armv9_shard_plan() -> Vec<String> {
    ARMV9_SHARD_ORDER.iter().map(|s| (*s).to_string()).collect()
}

/// Shardet mehrere benannte Dateien: erst Dateigrenzen, dann Extra-Splits
/// fuer grosse Dateien (`max_lines_per_chunk`).
///
/// `files`: `(name, contents)`-Paare. Jede Datei wird zuerst als Ganzes ein
/// Shard; enthaelt sie `file_start`-Marker, wird sie weiter unterteilt.
/// Grosse Dateien (siehe [`LARGE_INSTR_FILES`]) werden zusaetzlich per
/// [`extra_split`] in Chunks der Groesse `max_lines_per_chunk` zerlegt.
#[must_use]
pub fn shard_files(files: &[(String, String)], max_lines_per_chunk: usize) -> Vec<Shard> {
    let mut out = Vec::new();
    for (name, contents) in files {
        let mut parts = split_by_file_markers(contents);
        // Falls die Datei keine Marker enthaelt, traegt der Shard "default";
        // auf den uebergebenen Dateinamen umbenennen.
        if parts.len() == 1 && parts[0].id.name == "default" {
            parts[0].id = ShardId::new(name.clone(), 0);
        }
        for part in parts {
            if is_large_instr_file(&part.id.name) {
                out.extend(extra_split(&part, max_lines_per_chunk));
            } else {
                out.push(part);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundaries_from_markers() {
        let src = concat!(
            "// prolog\n",
            "pragma file_start \"instrs32.sail\"\n",
            "insn A\n",
            "pragma file_end\n",
            "pragma file_start \"sve.sail\"\n",
            "insn B\n",
            "pragma file_end\n",
        );
        let shards = split_by_file_markers(src);
        let names: Vec<String> = shards.iter().map(|s| s.id.to_string()).collect();
        assert_eq!(names, vec!["preamble", "instrs32", "sve"]);
        assert_eq!(shards[1].kind, ShardKind::File);
        assert!(shards[1].contents.contains("insn A"));
        assert!(shards[2].contents.contains("insn B"));
        // Lueckenlose Zeilenabdeckung.
        assert_eq!(shards[0].start_line, 1);
        assert_eq!(shards.last().unwrap().end_line, src.lines().count() + 1);
    }

    #[test]
    fn trailing_text_becomes_postamble() {
        let src = concat!(
            "pragma file_start \"a.sail\"\n",
            "body\n",
            "pragma file_end\n",
            "// kommentar danach\n",
        );
        let shards = split_by_file_markers(src);
        let names: Vec<String> = shards.iter().map(|s| s.id.to_string()).collect();
        assert_eq!(names, vec!["a", "postamble"]);
        assert_eq!(shards[1].kind, ShardKind::Synthetic);
        assert_eq!(shards[0].end_line, shards[1].start_line);
        assert_eq!(shards.last().unwrap().end_line, src.lines().count() + 1);
    }

    #[test]
    fn multiple_gaps_get_unique_ids() {
        let src = concat!(
            "pragma file_start \"a.sail\"\n",
            "x\n",
            "pragma file_end\n",
            "gap one\n",
            "pragma file_start \"b.sail\"\n",
            "y\n",
            "pragma file_end\n",
            "gap two\n",
        );
        let shards = split_by_file_markers(src);
        let names: Vec<String> = shards.iter().map(|s| s.id.to_string()).collect();
        assert_eq!(names, vec!["a", "postamble", "b", "postamble#1"]);
    }

    #[test]
    fn no_markers_single_default_shard() {
        let shards = split_by_file_markers("a\nb\n");
        assert_eq!(shards.len(), 1);
        assert_eq!(shards[0].id.name, "default");
    }

    #[test]
    fn unbalanced_marker_runs_to_eof() {
        let src = "pragma file_start \"x.sail\"\nbody\n";
        let shards = split_by_file_markers(src);
        assert_eq!(shards.len(), 1);
        assert_eq!(shards[0].id.name, "x");
        assert!(shards[0].contents.contains("body"));
    }

    #[test]
    fn extra_split_chunks_line_bounded() {
        let shard = Shard {
            id: ShardId::new("instrs32", 0),
            kind: ShardKind::File,
            start_line: 1,
            end_line: 11,
            contents: (0..10).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n"),
        };
        let chunks = extra_split(&shard, 4);
        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|c| c.line_count() <= 4));
        assert_eq!(chunks[0].id.to_string(), "instrs32");
        assert_eq!(chunks[1].id.to_string(), "instrs32#1");
        assert_eq!(chunks[2].id.to_string(), "instrs32#2");
        assert!(chunks.iter().all(|c| c.kind == ShardKind::SplitChunk));
        // Zeilenabdeckung lueckenlos, Inhalt rekonstruierbar.
        let joined = chunks
            .iter()
            .map(|c| c.contents.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(joined, shard.contents);
    }

    #[test]
    fn extra_split_small_shard_unchanged() {
        let shard = Shard {
            id: ShardId::new("misc", 0),
            kind: ShardKind::File,
            start_line: 1,
            end_line: 3,
            contents: "a\nb".to_string(),
        };
        let out = extra_split(&shard, 100);
        assert_eq!(out, vec![shard]);
    }

    #[test]
    fn large_file_detection() {
        for n in ["instrs32", "instrs64", "sve", "sme", "v8_base", "sysregs_autogen"] {
            assert!(is_large_instr_file(n), "{n}");
            assert!(is_large_instr_file(&format!("{n}.sail")), "{n}.sail");
        }
        assert!(!is_large_instr_file("decode"));
    }
}

//! Streaming-Zerlegung einer `.sail`-Datei in Top-Level-Definitionen.
//!
//! Lane-31-Regel: Es wird **nie** die Gesamt-Datei als ein Dokument im
//! Speicher gehalten. Der [`DefSplitter`] liest zeilenweise ueber ein
//! [`BufRead`] und liefert [`RawDef`]s (je genau eine Definition),
//! Bloecke gruppieren danach konstant viele Defs ([`BlockSplitter`]).
//!
//! Klammerzaehlung ignoriert `{`/`}` in `//`-Kommentaren,
//! `/*...*/`-Kommentaren, `"..."`-Strings und `'...'`-Chars, damit
//! Funktionsruempfe (z.B. die ~1900-zeiligen Reset-Funktionen in
//! `sysregs_autogen.sail`) exakt eine Def bleiben.

use std::io::{BufRead, Lines};

/// Sail-Top-Level-Starter, die eine neue Definition eroeffnen.
/// `sysregs_autogen.sail` nutzt nur `val`/`function`; der Rest macht den
/// Splitter fuer Fixtures und spaetere Shards wiederverwendbar.
const DEF_KEYWORDS: &[&str] = &[
    "val",
    "function",
    "mapping",
    "impl",
    "type",
    "struct",
    "union",
    "enum",
    "bitfield",
    "let",
    "constraint",
    "overload",
    "fixity",
    "default",
    "scattered",
    "register",
    "measure",
    "outcome",
    "instantiation",
];

/// Eine rohe Top-Level-Definition: Quelltext plus Startzeile (1-basiert).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDef {
    pub text: String,
    pub start_line: u32,
    pub end_line: u32,
}

impl RawDef {
    /// Erste Zeile ohne fuehrenden Whitespace (fuer Kind-/Namens-Extraktion).
    pub fn first_line(&self) -> &str {
        self.text.lines().next().unwrap_or("").trim_start()
    }
}

/// Fehler beim Streaming-Split (mit 1-basierter Zeilennummer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitError {
    pub line: u32,
    pub message: String,
}

impl SplitError {
    fn new(line: u32, message: impl Into<String>) -> Self {
        SplitError {
            line,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for SplitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "split error at line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for SplitError {}

/// Lexer-Zustand, der ueber Zeilen hinweg lebt (Blockkommentar,
/// String, Char-Literal). `line_comment` gilt nur bis Zeilenende.
#[derive(Debug, Default)]
struct ScanState {
    block_depth: u32,
    in_string: bool,
    in_char: bool,
}

/// Zaehlt `{`/`}` einer Zeile (Kommentare/Strings/Chars ignoriert) und
/// meldet, ob die Zeile Code ausserhalb davon enthaelt.
fn scan_line(line: &str, state: &mut ScanState) -> (i64, bool) {
    let bytes = line.as_bytes();
    let mut depth: i64 = 0;
    let mut has_code = false;
    let mut i = 0;
    // `escaped` lebt nur innerhalb String/Char und wird pro Zeile
    // zurueckgesetzt (Sail hat keine Multi-Line-Escapes am Zeilenende).
    let mut escaped = false;
    while i < bytes.len() {
        let b = bytes[i];
        if state.in_string {
            has_code = true;
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                state.in_string = false;
            }
            i += 1;
            continue;
        }
        if state.in_char {
            has_code = true;
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'\'' {
                state.in_char = false;
            }
            i += 1;
            continue;
        }
        if state.block_depth > 0 {
            if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                state.block_depth += 1;
                i += 2;
                continue;
            }
            if b == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                state.block_depth -= 1;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        // Normaler Code.
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            break; // Rest ist Zeilenkommentar.
        }
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            state.block_depth += 1;
            i += 2;
            continue;
        }
        if b == b'"' {
            state.in_string = true;
            has_code = true;
            i += 1;
            continue;
        }
        if b == b'\'' {
            state.in_char = true;
            has_code = true;
            i += 1;
            continue;
        }
        if !b.is_ascii_whitespace() {
            has_code = true;
        }
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
        }
        i += 1;
    }
    (depth, has_code)
}

/// `true`, wenn die Zeile (nach fuehrendem Whitespace, ausserhalb von
/// Kommentar/String gescannt) mit `keyword + Whitespace` beginnt.
fn starts_with_keyword(trimmed: &str, keyword: &str) -> bool {
    trimmed
        .strip_prefix(keyword)
        .is_some_and(|rest| rest.chars().next().is_some_and(|c| c.is_whitespace()))
}

/// Iterator ueber [`RawDef`]s aus einem [`BufRead`]-Stream.
/// Haelt nur die aktuelle Definition im Speicher, nie die Datei.
pub struct DefSplitter<R: BufRead> {
    lines: Lines<R>,
    line_no: u32,
    brace_depth: i64,
    state: ScanState,
    pending: Option<RawDef>,
    pending_start: u32,
    finished: bool,
    /// Ungueltige Klammerlage (mehr `}` als `{`) wird als Fehler gemeldet.
    errored: bool,
}

impl<R: BufRead> DefSplitter<R> {
    pub fn new(reader: R) -> Self {
        DefSplitter {
            lines: reader.lines(),
            line_no: 0,
            brace_depth: 0,
            state: ScanState::default(),
            pending: None,
            pending_start: 0,
            finished: false,
            errored: false,
        }
    }

    fn take_pending(&mut self, end_line: u32) -> Option<RawDef> {
        self.pending.take().map(|mut d| {
            d.end_line = end_line;
            d
        })
    }
}

impl<R: BufRead> Iterator for DefSplitter<R> {
    type Item = Result<RawDef, SplitError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished || self.errored {
            return None;
        }
        loop {
            let raw = match self.lines.next() {
                None => {
                    self.finished = true;
                    if self.state.block_depth > 0 {
                        return Some(Err(SplitError::new(
                            self.line_no,
                            "unterminated block comment at EOF",
                        )));
                    }
                    if self.brace_depth != 0 {
                        return Some(Err(SplitError::new(
                            self.line_no,
                            format!("unbalanced braces at EOF (depth {})", self.brace_depth),
                        )));
                    }
                    return self.take_pending(self.line_no).map(Ok);
                }
                Some(Err(e)) => {
                    self.errored = true;
                    return Some(Err(SplitError::new(
                        self.line_no + 1,
                        format!("i/o error: {e}"),
                    )));
                }
                Some(Ok(l)) => l,
            };
            self.line_no += 1;
            let (delta, has_code) = scan_line(&raw, &mut self.state);
            let trimmed = raw.trim_start();
            let is_def_start = self.brace_depth == 0
                && has_code
                && self.state.block_depth == 0
                && !self.state.in_string
                && DEF_KEYWORDS.iter().any(|k| starts_with_keyword(trimmed, k));

            if is_def_start {
                let done = self.take_pending(self.line_no - 1);
                self.pending_start = self.line_no;
                self.pending = Some(RawDef {
                    text: raw.clone(),
                    start_line: self.line_no,
                    end_line: self.line_no,
                });
                // Klammern der Startzeile selbst zaehlen (z.B. `... = {`).
                self.brace_depth += delta;
                if self.brace_depth < 0 {
                    self.errored = true;
                    return Some(Err(SplitError::new(self.line_no, "unbalanced `}`")));
                }
                if done.is_some() {
                    return done.map(Ok);
                }
                continue;
            }

            match self.pending.as_mut() {
                Some(def) => {
                    def.text.push('\n');
                    def.text.push_str(&raw);
                }
                None => {
                    // Prolog vor der ersten Definition (Lizenzheader,
                    // Leerzeilen, reine Kommentare): verwerfen.
                    if has_code {
                        self.errored = true;
                        return Some(Err(SplitError::new(
                            self.line_no,
                            format!("code outside any definition: {trimmed}"),
                        )));
                    }
                }
            }
            self.brace_depth += delta;
            if self.brace_depth < 0 {
                self.errored = true;
                return Some(Err(SplitError::new(self.line_no, "unbalanced `}`")));
            }
        }
    }
}

/// Ein Shard-Block: konstant viele [`RawDef`]s (nie das Gesamt-Dok).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub index: usize,
    pub defs: Vec<RawDef>,
}

impl Block {
    pub fn bytes(&self) -> usize {
        self.defs.iter().map(|d| d.text.len()).sum()
    }

    pub fn start_line(&self) -> u32 {
        self.defs.first().map(|d| d.start_line).unwrap_or(0)
    }

    pub fn end_line(&self) -> u32 {
        self.defs.last().map(|d| d.end_line).unwrap_or(0)
    }
}

/// Gruppiert einen [`DefSplitter`] in Bloecke zu je `defs_per_block` Defs.
/// Der letzte Block darf kleiner sein.
pub struct BlockSplitter<R: BufRead> {
    defs: DefSplitter<R>,
    defs_per_block: usize,
    index: usize,
    done: bool,
}

impl<R: BufRead> BlockSplitter<R> {
    pub fn new(reader: R, defs_per_block: usize) -> Self {
        assert!(defs_per_block > 0, "defs_per_block must be > 0");
        BlockSplitter {
            defs: DefSplitter::new(reader),
            defs_per_block,
            index: 0,
            done: false,
        }
    }
}

impl<R: BufRead> Iterator for BlockSplitter<R> {
    type Item = Result<Block, SplitError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let mut defs = Vec::with_capacity(self.defs_per_block);
        loop {
            match self.defs.next() {
                None => {
                    self.done = true;
                    if defs.is_empty() {
                        return None;
                    }
                    let block = Block {
                        index: self.index,
                        defs,
                    };
                    self.index += 1;
                    return Some(Ok(block));
                }
                Some(Err(e)) => {
                    self.done = true;
                    return Some(Err(e));
                }
                Some(Ok(def)) => {
                    defs.push(def);
                    if defs.len() >= self.defs_per_block {
                        let block = Block {
                            index: self.index,
                            defs,
                        };
                        self.index += 1;
                        return Some(Ok(block));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    fn split(src: &str) -> Result<Vec<RawDef>, SplitError> {
        DefSplitter::new(BufReader::new(src.as_bytes())).collect()
    }

    #[test]
    fn val_function_pair_is_two_defs() {
        let src = "val foo : int -> int\n\nfunction foo x = { x }\n";
        let defs = split(src).unwrap();
        assert_eq!(defs.len(), 2);
        assert!(defs[0].first_line().starts_with("val foo"));
        assert!(defs[1].first_line().starts_with("function foo"));
    }

    #[test]
    fn braces_in_comments_and_strings_do_not_split() {
        let src = concat!(
            "/* { license } */\n",
            "// trailing {\n",
            "function f x = {\n",
            "  let s = \"{ not a brace }\";\n",
            "  /* { still comment } */ x\n",
            "}\n",
        );
        let defs = split(src).unwrap();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].start_line, 3);
    }

    #[test]
    fn unbalanced_close_brace_errors() {
        let src = "function f x = { x }\n}\n";
        let err = split(src).unwrap_err();
        assert!(err.message.contains("unbalanced"), "{err}");
    }

    #[test]
    fn block_groups_const_defs() {
        let mut src = String::new();
        for i in 0..5 {
            src.push_str(&format!("val f{i} : int\n\nfunction f{i} x = {{ x }}\n"));
        }
        let blocks: Vec<Block> = BlockSplitter::new(BufReader::new(src.as_bytes()), 2)
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(blocks.len(), 5);
        assert_eq!(blocks[0].defs.len(), 2);
        assert_eq!(blocks[4].defs.len(), 2);
        assert_eq!(blocks[1].index, 1);
    }

    #[test]
    fn license_prolog_is_skipped() {
        let src = "/* license */\n\n\nval a : int\n";
        let defs = split(src).unwrap();
        assert_eq!(defs.len(), 1);
    }
}

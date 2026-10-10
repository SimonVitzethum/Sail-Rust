//! Pruefung eines Shard-Blocks (parsen im Lane-31-Sinn).
//!
//! Kein Typcheck, keine Namensaufloesung (spaetere Lanes): pro Block wird
//! nur validiert, was Sharding braucht —
//! * Kopf je Def (`val <name>` / `function <name>`, allgemein `kind name`),
//! * strikte `val`/`function`-Paarung mit Namensgleichheit (die
//!   sysregs-Datei besteht ausschliesslich aus solchen Paaren),
//! * Fingerprint (FNV-1a-64 ueber die Block-Bytes) fuer Lane 11
//!   (inkrementelles Ueberspringen unveraenderter Shards).

use crate::splitter::{Block, SplitError};

/// Gepruefter Blockkopf: Paarnamen in Quellreihenfolge plus Fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedBlock {
    pub index: usize,
    pub defs: usize,
    pub bytes: usize,
    pub start_line: u32,
    pub end_line: u32,
    /// Namen der `val`-Koepfe (== folgende `function`-Namen).
    pub names: Vec<String>,
    pub fingerprint: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockError {
    pub block: usize,
    pub message: String,
}

impl BlockError {
    fn new(block: usize, message: impl Into<String>) -> Self {
        BlockError {
            block,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for BlockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "block {}: {}", self.block, self.message)
    }
}

impl std::error::Error for BlockError {}

impl From<SplitError> for BlockError {
    fn from(e: SplitError) -> Self {
        BlockError::new(usize::MAX, e.message)
    }
}

/// FNV-1a-64 (lokal, ohne Deps; nur fuer Shard-Fingerprints).
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut h = OFFSET;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(PRIME);
    }
    h
}

/// Kopf einer Def: `(kind, name)`, z.B. `("val", "FOO_EL1_SysRegRead_…")`.
/// Mehrzeilige `val`-Signaturen (`val NAME : forall …` + Folgezeile)
/// spielen keine Rolle — nur die erste Zeile zaehlt.
pub fn def_head(first_line: &str) -> Option<(&str, &str)> {
    let mut words = first_line.split_whitespace();
    let kind = words.next()?;
    if !matches!(kind, "val" | "function") {
        return None;
    }
    let mut name = words.next()?;
    // `function NAME args...` / `val NAME : ...` — Namen enden an `(`, `:`, `,`.
    name = name.split(['(', ':', ',']).next().unwrap_or(name);
    if name.is_empty() {
        return None;
    }
    Some((kind, name))
}

/// Prueft genau einen Block: gerade Def-Anzahl, strikte
/// `val`/`function`-Abfolge, Namensgleichheit je Paar.
pub fn parse_block(block: &Block) -> Result<ParsedBlock, BlockError> {
    if block.defs.len() % 2 != 0 {
        return Err(BlockError::new(
            block.index,
            format!(
                "odd def count {} (expected val/function pairs)",
                block.defs.len()
            ),
        ));
    }
    let mut names = Vec::with_capacity(block.defs.len() / 2);
    let mut hash = 0xcbf29ce484222325u64;
    for pair in block.defs.chunks_exact(2) {
        let (val, fun) = (&pair[0], &pair[1]);
        let (vk, vn) = def_head(val.first_line()).ok_or_else(|| {
            BlockError::new(block.index, format!("bad val head: {:?}", val.first_line()))
        })?;
        let (fk, fn_) = def_head(fun.first_line()).ok_or_else(|| {
            BlockError::new(
                block.index,
                format!("bad function head: {:?}", fun.first_line()),
            )
        })?;
        if vk != "val" || fk != "function" {
            return Err(BlockError::new(
                block.index,
                format!("expected val/function pair, got {vk}/{fk}"),
            ));
        }
        if vn != fn_ {
            return Err(BlockError::new(
                block.index,
                format!("pair name mismatch: val {vn} vs function {fn_}"),
            ));
        }
        names.push(vn.to_string());
        // Fingerprint ueber beide Def-Texte (FNV-1a fortlaufend).
        for b in val.text.bytes().chain(fun.text.bytes()) {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    Ok(ParsedBlock {
        index: block.index,
        defs: block.defs.len(),
        bytes: block.bytes(),
        start_line: block.start_line(),
        end_line: block.end_line(),
        names,
        fingerprint: hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::splitter::{BlockSplitter, RawDef};
    use std::io::BufReader;

    fn blocks_of(src: &str, per: usize) -> Vec<Block> {
        BlockSplitter::new(BufReader::new(src.as_bytes()), per)
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn raw(text: &str, line: u32) -> RawDef {
        RawDef {
            text: text.to_string(),
            start_line: line,
            end_line: line,
        }
    }

    #[test]
    fn matching_pair_parses() {
        let src = "val FOO : int\n\nfunction FOO x = { x }\n";
        let blocks = blocks_of(src, 64);
        let p = parse_block(&blocks[0]).unwrap();
        assert_eq!(p.names, vec!["FOO".to_string()]);
        assert_eq!(p.defs, 2);
    }

    #[test]
    fn name_mismatch_errors() {
        let b = Block {
            index: 0,
            defs: vec![raw("val A : int", 1), raw("function B x = { x }", 3)],
        };
        assert!(parse_block(&b).is_err());
    }

    #[test]
    fn odd_def_count_errors() {
        let b = Block {
            index: 0,
            defs: vec![raw("val A : int", 1)],
        };
        assert!(parse_block(&b).is_err());
    }

    #[test]
    fn fnv_is_stable_and_sensitive() {
        assert_eq!(fnv1a64(b"abc"), fnv1a64(b"abc"));
        assert_ne!(fnv1a64(b"abc"), fnv1a64(b"abd"));
    }

    #[test]
    fn def_head_handles_params_and_sig() {
        assert_eq!(def_head("val FOO : forall 't. int"), Some(("val", "FOO")));
        assert_eq!(
            def_head("function FOO_EL1_SysRegRead_ab12 (el, op0) = {"),
            Some(("function", "FOO_EL1_SysRegRead_ab12"))
        );
        assert_eq!(def_head("}"), None);
    }
}

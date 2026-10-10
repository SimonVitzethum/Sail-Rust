//! Shard-Runner: Datei **streamen**, pro Block parsen, sofort verwerfen.
//!
//! RAM-Regel: Zu jedem Zeitpunkt lebt hoechstens **ein** Block
//! (`defs_per_block` Defs) plus ein `BufReader`-Puffer im Speicher —
//! niemals das Gesamt-Dokument. Pro Block werden nur konstante Metadaten
//! (Zeilen, Bytes, Fingerprint) plus der kombinierte Hash behalten.

use crate::block::{parse_block, BlockError};
use crate::rss;
use crate::splitter::BlockSplitter;
use std::fs::File;
use std::io::BufReader;
use std::time::Instant;

/// Shard-Konfiguration. `defs_per_block` muss gerade sein (`val`/`function`).
#[derive(Debug, Clone, Copy)]
pub struct ShardConfig {
    pub defs_per_block: usize,
}

impl Default for ShardConfig {
    fn default() -> Self {
        ShardConfig { defs_per_block: 64 }
    }
}

impl ShardConfig {
    pub fn validated(self) -> Result<Self, String> {
        if self.defs_per_block == 0 || self.defs_per_block % 2 != 0 {
            return Err(format!(
                "defs_per_block must be a positive even number, got {}",
                self.defs_per_block
            ));
        }
        Ok(self)
    }
}

/// Metadaten eines verarbeiteten Blocks (konstante Groesse).
#[derive(Debug, Clone)]
pub struct BlockStat {
    pub index: usize,
    pub defs: usize,
    pub bytes: usize,
    pub start_line: u32,
    pub end_line: u32,
    pub fingerprint: u64,
    /// RSS (KiB) direkt nach diesem Block — zeigt Wachstum pro Block.
    pub rss_kb_after: Option<u64>,
}

/// Ergebnis eines Shard-Laufs ueber genau eine Datei.
#[derive(Debug, Clone)]
pub struct ShardReport {
    pub path: String,
    pub defs_per_block: usize,
    pub blocks: Vec<BlockStat>,
    pub total_defs: usize,
    pub total_bytes: u64,
    pub total_lines: u32,
    pub elapsed_ms: u64,
    /// Peak-RSS (KiB, `VmHWM` am Ende) — die Lane-14-Kennzahl.
    pub peak_rss_kb: Option<u64>,
}

impl ShardReport {
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    pub fn max_block_bytes(&self) -> usize {
        self.blocks.iter().map(|b| b.bytes).max().unwrap_or(0)
    }

    /// Kombinierter Fingerprint aller Bloecke (FNV-1a ueber die Block-Hashes).
    pub fn combined_fingerprint(&self) -> u64 {
        let mut h = 0xcbf29ce484222325u64;
        for b in &self.blocks {
            for byte in b.fingerprint.to_le_bytes() {
                h ^= u64::from(byte);
                h = h.wrapping_mul(0x100000001b3);
            }
        }
        h
    }
}

/// Streamt `path` durch Splitter + Block-Parser. Gibt den Report zurueck;
/// Block-Quelltexte werden nach jedem Block verworfen.
pub fn run_shard(path: &str, config: ShardConfig) -> Result<ShardReport, BlockError> {
    let config = config.validated().map_err(|m| BlockError {
        block: usize::MAX,
        message: m,
    })?;
    let file = File::open(path).map_err(|e| BlockError {
        block: usize::MAX,
        message: format!("cannot open {path}: {e}"),
    })?;
    let total_bytes = file.metadata().map(|m| m.len()).unwrap_or(0);
    let reader = BufReader::with_capacity(64 << 10, file);
    let splitter = BlockSplitter::new(reader, config.defs_per_block);

    let start = Instant::now();
    let mut blocks = Vec::new();
    let mut total_defs = 0usize;
    let mut total_lines = 0u32;
    for item in splitter {
        let block = item?;
        total_lines = block.end_line();
        let parsed = parse_block(&block)?;
        // `block` (Quelltext) faellt hier aus dem Scope: verworfen.
        total_defs += parsed.defs;
        blocks.push(BlockStat {
            index: parsed.index,
            defs: parsed.defs,
            bytes: parsed.bytes,
            start_line: parsed.start_line,
            end_line: parsed.end_line,
            fingerprint: parsed.fingerprint,
            rss_kb_after: rss::current_rss_kb(),
        });
        drop(parsed);
    }
    let elapsed_ms = start.elapsed().as_millis() as u64;
    Ok(ShardReport {
        path: path.to_string(),
        defs_per_block: config.defs_per_block,
        blocks,
        total_defs,
        total_bytes,
        total_lines,
        elapsed_ms,
        peak_rss_kb: rss::peak_rss_kb(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_file(dir: &std::path::Path, name: &str, pairs: usize) -> String {
        let mut src = String::from("/* fixture */\n");
        for i in 0..pairs {
            src.push_str(&format!(
                "val F{i:03} : int\n\nfunction F{i:03} x = {{ x }}\n"
            ));
        }
        let path = dir.join(name);
        std::fs::write(&path, src).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn runner_counts_and_shards() {
        let dir = std::env::temp_dir().join("sail-sysregs-shard-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = fixture_file(&dir, "pairs.sail", 5);
        let report = run_shard(
            &path,
            ShardConfig { defs_per_block: 4 }.validated().unwrap(),
        )
        .unwrap();
        assert_eq!(report.total_defs, 10);
        assert_eq!(report.block_count(), 3); // 4+4+2 Defs
        assert_ne!(report.max_block_bytes(), 0);
        assert!(report.peak_rss_kb.unwrap_or(0) > 0);
        assert_eq!(report.blocks[0].defs, 4);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn odd_config_rejected() {
        assert!(ShardConfig { defs_per_block: 3 }.validated().is_err());
        assert!(ShardConfig { defs_per_block: 0 }.validated().is_err());
    }

    #[test]
    fn missing_file_errors() {
        let r = run_shard(
            "/nonexistent/sysregs.sail",
            ShardConfig::default().validated().unwrap(),
        );
        assert!(r.is_err());
    }

    #[test]
    fn reporter_combined_hash_stable() {
        let dir = std::env::temp_dir().join("sail-sysregs-shard-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = fixture_file(&dir, "stable.sail", 2);
        let cfg = ShardConfig { defs_per_block: 4 }.validated().unwrap();
        let a = run_shard(&path, cfg).unwrap();
        let b = run_shard(&path, cfg).unwrap();
        assert_eq!(a.combined_fingerprint(), b.combined_fingerprint());
        std::fs::remove_file(&path).ok();
    }
}

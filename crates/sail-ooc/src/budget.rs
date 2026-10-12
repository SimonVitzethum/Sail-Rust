//! Byte-Budget fuer Streaming-Phasen (Lane 22, Audit O-8).
//!
//! Das Definitionsfenster (`max_inflight` als reine Item-Zahl) skaliert nicht
//! mit der Definitionsgroesse: 2 riesige Bodies sprengen RAM, 2 winzige
//! lassen Worker hungern. Das Byte-Budget begrenzt stattdessen die Summe
//! gleichzeitig offener Eingabe-Bytes (Scheduler-Credits in Bytes, siehe
//! [`sail_shard::Scheduler::run_streaming_weighed`]).
//!
//! * [`parse_byte_limit`]: parst `"128M"`/`"512K"`/`"1G"`/reine Bytes.
//! * [`byte_budget_from_env`]: liest das Budget aus der Umgebung
//!   (`SAIL_TYPECHECK_MEMORY`), mit Default.
//! * Konstanten: [`DEFAULT_TYPECHECK_BUDGET_BYTES`],
//!   [`TYPECHECK_COUNT_CEILING`], [`TYPECHECK_BUDGET_ENV`].
//!
//! RAM-Schranke grosse Einzeldefinition: Ein Item groesser als das Budget
//! laeuft allein im leeren Fenster (kein Verhungern, kein Deadlock), liegt
//! dann aber voll in RAM (AST-Blob + Pruefzustand). Das Budget begrenzt die
//! *Summe* gleichzeitiger Items, nicht die Groesse eines einzelnen Items.
//! Wer einzelne Definitionen RAM-begrenzen will, muss vorab verkleinern
//! (OOC-Shard-Splits, `sail-shard` Lane 11) oder beim Aufrufer per
//! Blobgroesse ablehnen — der Scheduler dokumentiert hier nur das
//! Allein-Laufen statt Verklemmen (Test:
//! `weighed_streaming_lets_single_oversized_item_run_alone`).

use std::env;

/// Umgebungsvariable fuer das Typecheck-Byte-Budget (gleiche Variable, die
/// das Gate fuer das cgroup-Limit nutzt: Budget und Harte stimmen ueberein).
pub const TYPECHECK_BUDGET_ENV: &str = "SAIL_TYPECHECK_MEMORY";

/// Default-Budget, wenn [`TYPECHECK_BUDGET_ENV`] nicht gesetzt ist (32 MiB).
/// Das OOC-Gate prueft ein Vielfaches davon an AST-Bytes (Default 128 MiB),
/// sodass das Budget auch ohne Env kleiner als die Workload bleibt.
pub const DEFAULT_TYPECHECK_BUDGET_BYTES: u64 = 32 * 1024 * 1024;

/// Count-Safety-Deckel neben dem Byte-Budget (winzige Items): Der Scheduler
/// begrenzt gleichzeitig offene Items zusaetzlich auf diese Zahl.
pub const TYPECHECK_COUNT_CEILING: usize = 1024;

/// Fallback-Gewicht, wenn die Blobgroesse auf Platte nicht stat-bar ist
/// (Datei fehlt/verschwunden — der folgende Load scheitert ohnehin).
/// Konservativ klein, damit ein fehlender Stat das Fenster nicht blockiert.
pub const UNKNOWN_BLOB_BYTES: u64 = 64 * 1024;

/// Parst ein Byte-Limit: reine Zahl (Bytes) oder Zahl + Suffix `K`/`M`/`G`
/// (case-insensitiv, je 1024er-Potenzen), optional mit angehaengtem `B`
/// (`"128M"`, `"128MB"`, `"512K"`, `"1G"`, `"4096"`). Fuehrende/anhangende
/// Leerzeichen sind erlaubt. Fehler als lesbare Meldung (fuer CLI-Output).
pub fn parse_byte_limit(text: &str) -> Result<u64, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("empty byte limit".to_string());
    }
    let lower = text.to_ascii_lowercase();
    let lower = lower.strip_suffix('b').unwrap_or(&lower);
    let (digits, factor) = if let Some(n) = lower.strip_suffix('k') {
        (n, 1024u64)
    } else if let Some(n) = lower.strip_suffix('m') {
        (n, 1024u64 * 1024)
    } else if let Some(n) = lower.strip_suffix('g') {
        (n, 1024u64 * 1024 * 1024)
    } else {
        (lower, 1u64)
    };
    let digits = digits.trim();
    if digits.is_empty() {
        return Err(format!("invalid byte limit {text:?}: missing number"));
    }
    let value: u64 = digits
        .parse()
        .map_err(|_| format!("invalid byte limit {text:?}: expected <number>[K|M|G][B]"))?;
    let bytes = value
        .checked_mul(factor)
        .ok_or_else(|| format!("byte limit {text:?} overflows u64"))?;
    if bytes == 0 {
        return Err(format!("byte limit {text:?} must be positive"));
    }
    Ok(bytes)
}

/// Liest das Byte-Budget aus `var` (z. B. [`TYPECHECK_BUDGET_ENV`]);
/// nicht gesetzt oder leer => `default`. Parse-Fehler als lesbare Meldung.
pub fn byte_budget_from_env(var: &str, default: u64) -> Result<u64, String> {
    match env::var(var) {
        Err(env::VarError::NotPresent) => Ok(default),
        Err(e) => Err(format!("{var}: {e}")),
        Ok(text) if text.trim().is_empty() => Ok(default),
        Ok(text) => parse_byte_limit(&text).map_err(|e| format!("{var}: {e}")),
    }
}

/// Formatiert Bytes kompakt fuer Logs (`"32 MiB"`, `"512 KiB"`, `"123 B"`).
#[must_use]
pub fn format_bytes(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    const KIB: u64 = 1024;
    if bytes >= MIB && bytes.is_multiple_of(MIB) {
        format!("{} MiB", bytes / MIB)
    } else if bytes >= KIB && bytes.is_multiple_of(KIB) {
        format!("{} KiB", bytes / KIB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plain_and_suffixed_limits() {
        assert_eq!(parse_byte_limit("4096").unwrap(), 4096);
        assert_eq!(parse_byte_limit("512K").unwrap(), 512 * 1024);
        assert_eq!(parse_byte_limit("128M").unwrap(), 128 * 1024 * 1024);
        assert_eq!(parse_byte_limit("2G").unwrap(), 2 * 1024 * 1024 * 1024);
        assert_eq!(parse_byte_limit("128MB").unwrap(), 128 * 1024 * 1024);
        assert_eq!(parse_byte_limit(" 32m ").unwrap(), 32 * 1024 * 1024);
        assert_eq!(parse_byte_limit("1k").unwrap(), 1024);
    }

    #[test]
    fn parse_rejects_empty_zero_and_garbage() {
        assert!(parse_byte_limit("").is_err());
        assert!(parse_byte_limit("   ").is_err());
        assert!(parse_byte_limit("0").is_err());
        assert!(parse_byte_limit("0M").is_err());
        assert!(parse_byte_limit("abc").is_err());
        assert!(parse_byte_limit("12X").is_err());
        assert!(parse_byte_limit("M").is_err());
        assert!(parse_byte_limit("-4M").is_err());
    }

    #[test]
    fn budget_default_is_smaller_than_gate_workload() {
        // Gate-Workload: deutlich ueber 128 MiB AST-Bytes; Default-Budget 32 MiB.
        assert!(DEFAULT_TYPECHECK_BUDGET_BYTES < 128 * 1024 * 1024);
        assert_eq!(DEFAULT_TYPECHECK_BUDGET_BYTES, 32 * 1024 * 1024);
        assert!(TYPECHECK_COUNT_CEILING >= 256);
    }

    #[test]
    fn env_missing_or_blank_yields_default() {
        let var = "SAIL_OOC_LANE22_TEST_MISSING_VAR";
        env::remove_var(var);
        assert_eq!(byte_budget_from_env(var, 123).unwrap(), 123);
        env::set_var(var, "   ");
        assert_eq!(byte_budget_from_env(var, 123).unwrap(), 123);
        env::remove_var(var);
    }

    #[test]
    fn env_value_is_parsed_and_errors_named() {
        let var = "SAIL_OOC_LANE22_TEST_PARSE_VAR";
        env::set_var(var, "64M");
        assert_eq!(byte_budget_from_env(var, 1).unwrap(), 64 * 1024 * 1024);
        env::set_var(var, "kein-limit");
        let err = byte_budget_from_env(var, 1).expect_err("parse error");
        assert!(err.starts_with(var), "unexpected: {err}");
        env::remove_var(var);
    }

    #[test]
    fn format_bytes_roundtrips_typical_budgets() {
        assert_eq!(format_bytes(32 * 1024 * 1024), "32 MiB");
        assert_eq!(format_bytes(512 * 1024), "512 KiB");
        assert_eq!(format_bytes(123), "123 B");
    }
}

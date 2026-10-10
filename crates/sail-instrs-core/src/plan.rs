//! Shard-Zuordnung: welche Definition wohin gehört.
//!
//! Kern-Shards (Lane 33):
//!
//! | Shard      | Inhalt |
//! |------------|--------|
//! | `exec`     | alle `execute_*`-Paare (`val` + `function`) |
//! | `decode`   | alle `decode_*`-Paare (`val` + `function`) |
//! | `dispatch` | alle `function clause __Decode…` (Dispatch-Arme) |
//! | `helper`   | alles Übrige (`v8_base.sail`: Register, Typen, Hilfsfunktionen) |
//!
//! Externe Anker (nicht in diesen Dateien, aber Shard-Abhängigkeiten):
//! `scattered function __Decode…`-Deklarationen (`decode_start.sail`),
//! Default-Arme + Top-Dispatch `__DecodeA64/__DecodeA32` (`decode_end.sail`).

use crate::scanner::{DefHeader, DefKind};

/// Ziel-Shard einer Definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shard {
    /// `execute_*`-Semantik.
    Exec,
    /// `decode_*`-Vorstufe (Feld-Extraktion + Guards, ruft `execute_*`).
    Decode,
    /// `function clause __Decode…` (Bitmuster-Dispatch mit `SEE`-Priorität).
    Dispatch,
    /// Helfer (Register, Typen, reine Funktionen aus `v8_base.sail`).
    Helper,
}

impl Shard {
    /// Stabile Shard-Bezeichnung für Reports und Fingerprints (Lane 11).
    pub fn as_str(self) -> &'static str {
        match self {
            Shard::Exec => "exec",
            Shard::Decode => "decode",
            Shard::Dispatch => "dispatch",
            Shard::Helper => "helper",
        }
    }
}

/// Ordnet eine Kopfzeile ihrem Shard zu.
///
/// Regel: `execute_*` → [`Shard::Exec`], `decode_*` → [`Shard::Decode`],
/// `function clause` → [`Shard::Dispatch`], Rest → [`Shard::Helper`].
/// `val`-Signaturen erben den Shard ihrer Funktion über den Namenspräfix.
pub fn shard_of(header: &DefHeader) -> Shard {
    if header.kind == DefKind::Clause {
        return Shard::Dispatch;
    }
    if header.name.starts_with("execute_") {
        return Shard::Exec;
    }
    if header.name.starts_with("decode_") {
        return Shard::Decode;
    }
    Shard::Helper
}

/// Gezählte Shard-Belegung einer Datei.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilePlan {
    pub exec: u64,
    pub decode: u64,
    pub dispatch: u64,
    pub helper: u64,
}

impl FilePlan {
    /// Faltet eine Kopfzeile in den Plan.
    pub fn add(&mut self, header: &DefHeader) {
        match shard_of(header) {
            Shard::Exec => self.exec += 1,
            Shard::Decode => self.decode += 1,
            Shard::Dispatch => self.dispatch += 1,
            Shard::Helper => self.helper += 1,
        }
    }

    /// Gesamtzahl zugeordneter Definitionen.
    pub fn total(&self) -> u64 {
        self.exec + self.decode + self.dispatch + self.helper
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(kind: DefKind, name: &str) -> DefHeader {
        DefHeader { kind, name: name.to_string(), line: 1 }
    }

    #[test]
    fn val_inherits_shard_from_name() {
        assert_eq!(shard_of(&head(DefKind::Val, "execute_foo")), Shard::Exec);
        assert_eq!(shard_of(&head(DefKind::Val, "decode_foo")), Shard::Decode);
        assert_eq!(shard_of(&head(DefKind::Val, "X_read")), Shard::Helper);
    }

    #[test]
    fn clause_always_dispatch() {
        assert_eq!(shard_of(&head(DefKind::Clause, "__DecodeT32")), Shard::Dispatch);
    }

    #[test]
    fn helpers_catch_the_rest() {
        for name in ["Feature", "PSTATE", "AddWithCarry", "X_read", "R_set"] {
            assert_eq!(shard_of(&head(DefKind::Function, name)), Shard::Helper);
            assert_eq!(shard_of(&head(DefKind::Other, name)), Shard::Helper);
        }
    }

    #[test]
    fn plan_counts_add_up() {
        let mut p = FilePlan::default();
        p.add(&head(DefKind::Val, "execute_a"));
        p.add(&head(DefKind::Function, "execute_a"));
        p.add(&head(DefKind::Function, "decode_a"));
        p.add(&head(DefKind::Clause, "__DecodeA64"));
        p.add(&head(DefKind::Other, "PSTATE"));
        assert_eq!((p.exec, p.decode, p.dispatch, p.helper), (2, 1, 1, 1));
        assert_eq!(p.total(), 5);
    }
}

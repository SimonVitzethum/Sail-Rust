#![forbid(unsafe_code)]
//! Arch-Instanz-Generierung. Spiegel von `doc_instantiations_v1/v2`
//! (`sail/src/sail_lean_backend/pretty_print_lean.ml:1524-1605`).
//!
//! Die Emitter erzeugen ausschließlich den `instance : Arch where …`-Block;
//! Monaden-Abkürzungen (`SailM`/`SailME`) und Register-`Inhabited`-Instanzen
//! gehören zu anderen Lanes. Kein Multicore-Emulator.

/// V1-Parameter. Spiegel von `doc_instantiations_v1` (`pretty_print_lean.ml`
/// Zeilen ~1524-1553): `va_size` ist fest `64`; die restlichen Felder stammen
/// aus `Monad_params.find_monad_parameters` (`sail/src/lib/monad_params.ml`).
/// Fehlt `sail_mem_read` (`params = None`), emittiert der Master nichts
/// (`None -> empty`); das bildet [`V1Params::emit`] mit `None` ab.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct V1Params {
    pub pa: String,
    pub abort: String,
    pub translation: String,
    pub trans_start: String,
    pub trans_end: String,
    pub fault: String,
    pub tlb_op: String,
    pub cache_op: String,
    pub barrier: String,
    pub arch_ak: String,
    pub sys_reg_id: String,
}

impl V1Params {
    /// Emittiert den `instance : Arch where`-Block (V1-Feldnamen:
    /// `va_size/pa/abort/translation/trans_start/trans_end/fault/tlb_op/
    /// cache_op/barrier/arch_ak/sys_reg_id`).
    #[must_use]
    pub fn emit(&self) -> String {
        let mut out = String::from("instance : Arch where\n");
        out.push_str("  va_size := 64\n");
        out.push_str(&format!("  pa := {}\n", self.pa));
        out.push_str(&format!("  abort := {}\n", self.abort));
        out.push_str(&format!("  translation := {}\n", self.translation));
        out.push_str(&format!("  trans_start := {}\n", self.trans_start));
        out.push_str(&format!("  trans_end := {}\n", self.trans_end));
        out.push_str(&format!("  fault := {}\n", self.fault));
        out.push_str(&format!("  tlb_op := {}\n", self.tlb_op));
        out.push_str(&format!("  cache_op := {}\n", self.cache_op));
        out.push_str(&format!("  barrier := {}\n", self.barrier));
        out.push_str(&format!("  arch_ak := {}\n", self.arch_ak));
        out.push_str(&format!("  sys_reg_id := {}\n", self.sys_reg_id));
        out
    }
}

/// V2-Instanz. Spiegel von `doc_instantiations_v2` (`pretty_print_lean.ml`
/// Zeilen ~1555-1600): Typ-Substitutionen stammen aus Sail-`instantiation`s
/// (`Monad_params.find_instantiations`), Ident-Substitutionen ebenso.
/// Fehlende Typen fallen auf den Default `d` zurück, fehlende Funktionen auf
/// `fun _ => false` — exakt wie `ts`/`is` im Master.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct V2Instance {
    /// `(kid, Lean-Typ)`-Paare, z.B. `("addr_size", "64")`.
    pub type_substs: Vec<(String, String)>,
    /// `(sail-id, Lean-Ctor)`-Paare, z.B. `("mem_acc_is_explicit", "isExplicit")`.
    pub id_substs: Vec<(String, String)>,
}

impl V2Instance {
    #[must_use]
    pub fn new(
        type_substs: Vec<(String, String)>,
        id_substs: Vec<(String, String)>,
    ) -> Self {
        Self { type_substs, id_substs }
    }

    fn ty(&self, kid: &str, default: &str) -> String {
        self.type_substs
            .iter()
            .find(|(k, _)| k == kid)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| default.to_string())
    }

    fn fun_(&self, id: &str) -> String {
        self.id_substs
            .iter()
            .find(|(k, _)| k == id)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| "fun _ => false".to_string())
    }

    /// Emittiert den `@[reducible] instance : Arch where`-Block mit allen 22
    /// Feldern in Master-Reihenfolge (`addr_size` … `sys_reg_id`).
    #[must_use]
    pub fn emit(&self) -> String {
        let mut out = String::from("@[reducible]\ninstance : Arch where\n");
        let tys: &[(&str, &str)] = &[
            ("addr_size", "64"),
            ("addr_space", "Unit"),
            ("CHERI", "false"),
            ("cap_size_log", "0"),
            ("register", "Register"),
            ("register_type", "RegisterType"),
            ("mem_acc", "Unit"),
            ("trans_start", "Unit"),
            ("trans_end", "Unit"),
            ("abort", "Unit"),
            ("barrier", "Unit"),
            ("cache_op", "Unit"),
            ("tlbi", "Unit"),
            ("exn", "Unit"),
            ("sys_reg_id", "Unit"),
        ];
        for (kid, default) in tys {
            if *kid == "mem_acc" {
                out.push_str(&format!("  mem_acc := {}\n", self.ty(kid, default)));
                for f in [
                    "mem_acc_is_explicit",
                    "mem_acc_is_ifetch",
                    "mem_acc_is_ttw",
                    "mem_acc_is_relaxed",
                    "mem_acc_is_rel_acq_rcpc",
                    "mem_acc_is_rel_acq_rcsc",
                    "mem_acc_is_standalone",
                    "mem_acc_is_exclusive",
                    "mem_acc_is_atomic_rmw",
                ] {
                    out.push_str(&format!("  {f} := {}\n", self.fun_(f)));
                }
            } else {
                out.push_str(&format!("  {kid} := {}\n", self.ty(kid, default)));
            }
        }
        out
    }

    /// Minimale `tiny-arm`-Instanz (Lane-29-Done-Kriterium „Arch-Instanz baut"):
    /// 64-Bit-Adressen, eigene Register-/MemAcc-Typen, Rest `Unit`.
    #[must_use]
    pub fn tiny_arm() -> Self {
        Self::new(
            vec![
                ("addr_size".to_string(), "64".to_string()),
                ("addr_space".to_string(), "TinyAddrSpace".to_string()),
                ("register".to_string(), "TinyReg".to_string()),
                ("register_type".to_string(), "TinyRegType".to_string()),
                ("mem_acc".to_string(), "TinyMemAcc".to_string()),
                ("barrier".to_string(), "TinyBarrier".to_string()),
            ],
            vec![
                ("mem_acc_is_explicit".to_string(), "TinyMemAcc.isExplicit".to_string()),
                ("mem_acc_is_exclusive".to_string(), "TinyMemAcc.isExclusive".to_string()),
            ],
        )
    }
}

#![forbid(unsafe_code)]
//! V1-Concurrency-Interface. Spiegel von `Sail.ConcurrencyInterfaceV1`
//! (`lean-sail/Sail/ConcurrencyInterfaceV1.lean`).
//!
//! V1 nutzt eigene `Access_kind`-/`Mem_read/write_request`-Typen
//! (`ConcurrencyInterfaceV1.lean:41-91`) und eine direkte
//! EStateM-Auswertung (`:195-290`).

use std::collections::HashMap;

use crate::v2::SeqError;

/// Zugriffs-Varietät. Spiegel von `Access_variety` (`:41-45`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessVariety {
    Plain,
    Exclusive,
    AtomicRmw,
}

impl AccessVariety {
    /// Lean-Konstruktorname.
    #[must_use]
    pub const fn lean_ctor(&self) -> &'static str {
        match self {
            Self::Plain => "AV_plain",
            Self::Exclusive => "AV_exclusive",
            Self::AtomicRmw => "AV_atomic_rmw",
        }
    }
}

/// Zugriffs-Stärke. Spiegel von `Access_strength` (`:47-55`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessStrength {
    Normal,
    RelOrAcq,
    AcqRcpc,
}

impl AccessStrength {
    /// Lean-Konstruktorname.
    #[must_use]
    pub const fn lean_ctor(&self) -> &'static str {
        match self {
            Self::Normal => "AS_normal",
            Self::RelOrAcq => "AS_rel_or_acq",
            Self::AcqRcpc => "AS_acq_rcpc",
        }
    }
}

/// Explizite Zugriffsart. Spiegel von `Explicit_access_kind` (`:57-60`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExplicitAccessKind {
    pub variety: AccessVariety,
    pub strength: AccessStrength,
}

/// Zugriffsart. Spiegel von `Access_kind` (`:62-70`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessKind<ArchAk> {
    Explicit(ExplicitAccessKind),
    Ifetch,
    Ttw,
    Arch(ArchAk),
}

impl<ArchAk> AccessKind<ArchAk> {
    /// Lean-Konstruktorname.
    #[must_use]
    pub const fn lean_ctor(&self) -> &'static str {
        match self {
            Self::Explicit(_) => "AK_explicit",
            Self::Ifetch => "AK_ifetch",
            Self::Ttw => "AK_ttw",
            Self::Arch(_) => "AK_arch",
        }
    }
}

/// Lese-Request. Spiegel von `Mem_read_request`
/// (`ConcurrencyInterfaceV1.lean:72-80`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemReadRequest<ArchAk, Pa, Ts> {
    pub access_kind: AccessKind<ArchAk>,
    /// `va : Option (BitVec vasize)`.
    pub va: Option<u64>,
    pub pa: Pa,
    pub translation: Ts,
    /// `size : Int` im Master; hier `i64`.
    pub size: i64,
    pub tag: bool,
}

/// Schreib-Request. Spiegel von `Mem_write_request` (`:82-91`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemWriteRequest<ArchAk, Pa, Ts> {
    pub access_kind: AccessKind<ArchAk>,
    pub va: Option<u64>,
    pub pa: Pa,
    pub translation: Ts,
    /// `size : Int` im Master; hier `i64`.
    pub size: i64,
    /// `value : Option (BitVec (8*n))`, als Bytes little-endian.
    pub value: Option<Vec<u8>>,
    pub tag: Option<bool>,
}

/// V1-Arch-Parameter. Spiegel von `class Arch` (`:27-39`).
pub trait Arch {
    /// `va_size`.
    const VA_SIZE: usize;
    type Pa: Clone + core::fmt::Debug;
    type ArchAk: Clone + core::fmt::Debug;
    type Translation: Clone + core::fmt::Debug;
    type TransStart: Clone + core::fmt::Debug;
    type TransEnd: Clone + core::fmt::Debug;
    type Abort: Clone + PartialEq + core::fmt::Debug;
    type Barrier: Clone + core::fmt::Debug;
    type CacheOp: Clone + core::fmt::Debug;
    type TlbOp: Clone + core::fmt::Debug;
    type Fault: Clone + core::fmt::Debug;
    type SysRegId: Clone + core::fmt::Debug;
}

/// Sequentieller V1-Zustand. Spiegel von `SequentialState` (`:101-107`).
#[derive(Debug, Clone, Default)]
pub struct SequentialState<Reg, RegVal> {
    pub regs: HashMap<Reg, RegVal>,
    /// Triviale Choice-Quelle (`trivialChoiceSource`, `:14-26`) braucht keinen
    /// Zustand; der Zähler dokumentiert die erfolgte Auswahl deterministisch.
    pub choice_count: u64,
    pub mem: HashMap<u64, u8>,
    pub cycle_count: u64,
    pub output: Vec<String>,
}

impl<Reg: Eq + core::hash::Hash, RegVal: Clone> SequentialState<Reg, RegVal> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            regs: HashMap::new(),
            choice_count: 0,
            mem: HashMap::new(),
            cycle_count: 0,
            output: Vec::new(),
        }
    }

    /// Spiegel von `writeBytes` (`:200-203`): little-endian, gibt `true` zurück.
    pub fn write_bytes(&mut self, addr: u64, value: &[u8]) -> bool {
        for (i, b) in value.iter().enumerate() {
            self.mem.insert(addr + i as u64, *b);
        }
        true
    }

    /// Spiegel von `readBytes` (`:223-235`): Byte an `addr` ist LSB.
    pub fn read_bytes(&self, size: usize, addr: u64) -> Result<(Vec<u8>, Option<bool>), SeqError> {
        let mut out = Vec::with_capacity(size);
        for i in 0..size {
            let a = addr + i as u64;
            out.push(self.mem.get(&a).copied().ok_or(SeqError::OutOfMemoryRange(a))?);
        }
        Ok((out, None))
    }

    /// Spiegel von `sail_mem_write` (`:251-256`): `none` schreibt nichts,
    /// meldet aber Erfolg; Ergebnis `Ok (some b)`.
    pub fn mem_write(&mut self, addr: u64, value: Option<&[u8]>) -> bool {
        match value {
            Some(v) => self.write_bytes(addr, v),
            None => true,
        }
    }

    /// Spiegel von `sail_mem_read` (`:259-262`).
    pub fn mem_read(&self, size: usize, addr: u64) -> Result<(Vec<u8>, Option<bool>), SeqError> {
        self.read_bytes(size, addr)
    }

    /// No-op-Arme: `sail_barrier/sail_cache_op/sail_tlbi/sail_translation_*`
    /// (`:265-273`) sind im sequentiellen Master `pure ()`.
    pub fn barrier(&mut self) {}
    pub fn cache_op(&mut self) {}
    pub fn tlbi(&mut self) {}
    pub fn translation_start(&mut self) {}
    pub fn translation_end(&mut self) {}
    pub fn take_exception(&mut self) {}
    pub fn return_exception(&mut self) {}

    /// Spiegel von `cycle_count` / `get_cycle_count` (`:285-290`).
    pub fn cycle_count(&mut self) {
        self.cycle_count += 1;
    }

    #[must_use]
    pub fn get_cycle_count(&self) -> u64 {
        self.cycle_count
    }
}

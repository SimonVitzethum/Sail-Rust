#![forbid(unsafe_code)]
//! V2-Arch-Interface. Spiegel von `Sail.ArchSem` (`ArchSem.lean`).
//!
//! Feldreihenfolge und Defaults folgen dem Master 1:1; siehe die
//! Quellenverweise an den einzelnen Items.

use std::collections::HashMap;

/// Architekturparameter. Spiegel von `class Arch` (`ArchSem.lean:8-59`).
///
/// Assoziierte Funktionen `is_*` spiegeln die `mem_acc_is_*`-Felder
/// (`ArchSem.lean:30-40`); [`Arch::is_rel_acq`] spiegelt die Default-Definition
/// `mem_acc_is_rel_acq_rcpc || mem_acc_is_rel_acq_rcsc` (`ArchSem.lean:36-37).
pub trait Arch {
    /// `addr_size` (`ArchSem.lean:10`).
    const ADDR_SIZE: usize;
    /// `CHERI` (`ArchSem.lean:17`).
    const CHERI: bool;
    /// `cap_size_log` (`ArchSem.lean:22`).
    const CAP_SIZE_LOG: usize;

    type AddrSpace: Clone + PartialEq + core::fmt::Debug;
    type Register: Clone + PartialEq + Eq + core::hash::Hash + core::fmt::Debug;
    type RegisterValue: Clone + PartialEq + core::fmt::Debug;
    type MemAcc: Clone + PartialEq + core::fmt::Debug;
    type TransStart: Clone + core::fmt::Debug;
    type TransEnd: Clone + core::fmt::Debug;
    type Abort: Clone + PartialEq + core::fmt::Debug;
    type Barrier: Clone + core::fmt::Debug;
    type CacheOp: Clone + core::fmt::Debug;
    type Tlbi: Clone + core::fmt::Debug;
    type Exn: Clone + core::fmt::Debug;
    type SysRegId: Clone + core::fmt::Debug;

    fn mem_acc_is_explicit(acc: &Self::MemAcc) -> bool;
    fn mem_acc_is_ifetch(acc: &Self::MemAcc) -> bool;
    fn mem_acc_is_ttw(acc: &Self::MemAcc) -> bool;
    fn mem_acc_is_relaxed(acc: &Self::MemAcc) -> bool;
    fn mem_acc_is_rel_acq_rcpc(acc: &Self::MemAcc) -> bool;
    fn mem_acc_is_rel_acq_rcsc(acc: &Self::MemAcc) -> bool;
    /// Default: `rcpc || rcsc` (`ArchSem.lean:36-37`). Muss nicht überschrieben werden.
    fn mem_acc_is_rel_acq(acc: &Self::MemAcc) -> bool {
        Self::mem_acc_is_rel_acq_rcpc(acc) || Self::mem_acc_is_rel_acq_rcsc(acc)
    }
    fn mem_acc_is_standalone(acc: &Self::MemAcc) -> bool;
    fn mem_acc_is_exclusive(acc: &Self::MemAcc) -> bool;
    fn mem_acc_is_atomic_rmw(acc: &Self::MemAcc) -> bool;
}

/// Externer Memory-Request. Spiegel von `structure MemRequest` (`ArchSem.lean:66-71`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemRequest<Acc, Space> {
    /// `accessKind`.
    pub access_kind: Acc,
    /// Adresse als `u128`, gültig sind die unteren [`Arch::ADDR_SIZE`] Bits
    /// (Spiegel von `BitVec Arch.addr_size`).
    pub address: u128,
    /// `addressSpace`.
    pub address_space: Space,
    /// `size` (Byte).
    pub size: usize,
    /// `numTag`.
    pub num_tag: usize,
}

impl<Acc, Space> MemRequest<Acc, Space> {
    /// Maskiert die Adresse auf `addr_size` Bits (Spiegel der `BitVec`-Trunkierung).
    #[must_use]
    pub fn masked_address(&self, addr_size: usize) -> u128 {
        if addr_size >= 128 {
            self.address
        } else if addr_size == 0 {
            0
        } else {
            self.address & ((1u128 << addr_size) - 1)
        }
    }
}

/// Sail-interner Request mit Size/Tag auf Term-Ebene. Spiegel von
/// `structure Mem_request` (`ArchSem.lean:80-86`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SailMemRequest<Acc, Space> {
    /// Term-Ebene `size` (spiegelt Typ-Parameter `_size`).
    pub size: usize,
    /// Term-Ebene `num_tag` (spiegelt Typ-Parameter `_numTags`).
    pub num_tag: usize,
    /// `addr_size` des Modells (spiegelt Typ-Parameter `addr_size`).
    pub addr_size: usize,
    pub access_kind: Acc,
    pub address: u128,
    pub address_space: Space,
    /// Redundante Term-Ebene `size` (wie im Lean-Struct Feld `size`).
    pub size_field: usize,
    /// Redundante Term-Ebene `num_tag` (wie im Lean-Struct Feld `num_tag`).
    pub num_tag_field: usize,
}

impl<Acc: Clone, Space: Clone> SailMemRequest<Acc, Space> {
    /// Spiegel von `Mem_request.toArchSem` (`ArchSem.lean:89-97`):
    /// `size`/`numTag` kommen aus den Typ-Parametern (hier: `size`/`num_tag`),
    /// Rest wird feldweise kopiert.
    #[must_use]
    pub fn to_archsem(&self) -> MemRequest<Acc, Space> {
        MemRequest {
            access_kind: self.access_kind.clone(),
            address: self.address,
            address_space: self.address_space.clone(),
            size: self.size,
            num_tag: self.num_tag,
        }
    }
}

/// ISA-Effekt. Spiegel von `inductive InstructionEffect` (`ArchSem.lean:157-172`).
///
/// Genau 15 Konstruktoren, gleiche Namen/Reihenfolge wie im Master.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstructionEffect<Acc, Space, Reg, RegVal, Ts, Te, B, C, T, E, S> {
    RegRead { reg: Reg, access: Option<S> },
    RegWrite { reg: Reg, access: Option<S>, value: RegVal },
    MemRead(MemRequest<Acc, Space>),
    MemWrite { req: MemRequest<Acc, Space>, value: Vec<u8>, tags: Vec<bool> },
    MemWriteAnnounce(MemRequest<Acc, Space>),
    Barrier(B),
    CacheOp(C),
    TlbOp(T),
    ClockCycle,
    GetCycleCount,
    TranslationStart(Ts),
    TranslationEnd(Te),
    ArchException(E),
    ReturnException,
    PrintMessage(String),
}

impl<Acc, Space, Reg, RegVal, Ts, Te, B, C, T, E, S> InstructionEffect<Acc, Space, Reg, RegVal, Ts, Te, B, C, T, E, S> {
    /// Lean-Konstruktorname (`ArchSem.lean:157-172`). Für Difftests.
    #[must_use]
    pub const fn lean_ctor(&self) -> &'static str {
        match self {
            Self::RegRead { .. } => "regRead",
            Self::RegWrite { .. } => "regWrite",
            Self::MemRead(_) => "memRead",
            Self::MemWrite { .. } => "memWrite",
            Self::MemWriteAnnounce(_) => "memWriteAnnounce",
            Self::Barrier(_) => "barrier",
            Self::CacheOp(_) => "cacheOp",
            Self::TlbOp(_) => "tlbOp",
            Self::ClockCycle => "clockCycle",
            Self::GetCycleCount => "getCycleCount",
            Self::TranslationStart(_) => "translationStart",
            Self::TranslationEnd(_) => "translationEnd",
            Self::ArchException(_) => "archException",
            Self::ReturnException => "returnException",
            Self::PrintMessage(_) => "printMessage",
        }
    }

    /// Rückgabetyp-Beschreibung. Spiegel von `InstructionEffect.ret`
    /// (`ArchSem.lean:174-189`).
    #[must_use]
    pub fn ret_desc(&self) -> RetDesc {
        match self {
            Self::RegRead { .. } => RetDesc::RegisterValue,
            Self::RegWrite { .. }
            | Self::MemWriteAnnounce(_)
            | Self::Barrier(_)
            | Self::CacheOp(_)
            | Self::TlbOp(_)
            | Self::ClockCycle
            | Self::TranslationStart(_)
            | Self::TranslationEnd(_)
            | Self::ArchException(_)
            | Self::ReturnException
            | Self::PrintMessage(_) => RetDesc::Unit,
            Self::MemRead(req) => RetDesc::MemReadResult {
                size: req.size,
                num_tag: req.num_tag,
            },
            Self::MemWrite { .. } => RetDesc::MemWriteResult,
            Self::GetCycleCount => RetDesc::Nat,
        }
    }
}

/// Rückgabetyp eines [`InstructionEffect`], Spiegel von
/// `InstructionEffect.ret` (`ArchSem.lean:174-189`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetDesc {
    /// `Arch.register_type reg` (regRead).
    RegisterValue,
    /// `Unit` (alle reinen No-op-/Steuer-Effekte).
    Unit,
    /// `Except abort (BitVec (8*size) × BitVec numTag)` (memRead).
    MemReadResult {
        size: usize,
        num_tag: usize,
    },
    /// `Except abort Unit` (memWrite).
    MemWriteResult,
    /// `Nat` (getCycleCount).
    Nat,
}

/// Fehler der sequentiellen Auswertung. Spiegel der in
/// `interpretInstructionEffect` (`ArchSemSequential.lean:52-79`) beobachtbaren
/// `Sail.Error`-Fälle: `OutOfMemoryRange`, `Assertion`, `Unreachable`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeqError {
    OutOfMemoryRange(u64),
    Assertion(String),
    Unreachable,
}

impl core::fmt::Display for SeqError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::OutOfMemoryRange(a) => write!(f, "out of memory range: {a}"),
            Self::Assertion(s) => write!(f, "assertion failed: {s}"),
            Self::Unreachable => write!(f, "unreachable"),
        }
    }
}

impl std::error::Error for SeqError {}

/// Sequentieller Zustand. Spiegel von `SequentialState`
/// (`ArchSemSequential.lean:17-23`). Keine eigene Semantik: reine
/// Single-Thread-Spiegelung des Masters (kein Multicore-Emulator).
#[derive(Debug, Clone, Default)]
pub struct SequentialState<Reg, RegVal> {
    pub regs: HashMap<Reg, RegVal>,
    /// Byte-Adressiertes Gedächtnis (`mem : HashMap Nat (BitVec 8)`).
    pub mem: HashMap<u64, u8>,
    pub cycle_count: u64,
    /// `sailOutput`.
    pub output: Vec<String>,
}

impl<Reg: Eq + core::hash::Hash, RegVal: Clone> SequentialState<Reg, RegVal> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            regs: HashMap::new(),
            mem: HashMap::new(),
            cycle_count: 0,
            output: Vec::new(),
        }
    }

    /// Spiegel von `writeByte` (`ArchSemSequential.lean:27-28`).
    pub fn write_byte(&mut self, addr: u64, value: u8) {
        self.mem.insert(addr, value);
    }

    /// Spiegel von `writeBytes` (`ArchSemSequential.lean:30-32`):
    /// Byte `i` = `extractLsb' (8*i) 8` (little-endian).
    pub fn write_bytes(&mut self, addr: u64, value: &[u8]) {
        for (i, b) in value.iter().enumerate() {
            self.write_byte(addr + i as u64, *b);
        }
    }

    /// Spiegel von `readByte` (`ArchSemSequential.lean:34-37`):
    /// fehlende Adresse → `OutOfMemoryRange`.
    pub fn read_byte(&self, addr: u64) -> Result<u8, SeqError> {
        self.mem
            .get(&addr)
            .copied()
            .ok_or(SeqError::OutOfMemoryRange(addr))
    }

    /// Spiegel von `readBytes` (`ArchSemSequential.lean:39-50`): Byte an
    /// `addr` ist LSB (`bytes.append b`).
    pub fn read_bytes(&self, size: usize, addr: u64) -> Result<Vec<u8>, SeqError> {
        (0..size)
            .map(|i| self.read_byte(addr + i as u64))
            .collect()
    }

    /// Spiegel von `interpretInstructionEffect` für `.memRead`
    /// (`ArchSemSequential.lean:59-63`): getaggter Speicher wird mit
    /// `Assertion "Tagged memory not supported"` abgelehnt.
    pub fn mem_read(
        &self,
        req: &MemRequest<impl Clone, impl Clone>,
        addr: u64,
    ) -> Result<(Vec<u8>, Vec<bool>), SeqError> {
        if req.num_tag != 0 {
            return Err(SeqError::Assertion("Tagged memory not supported".to_string()));
        }
        let bytes = self.read_bytes(req.size, addr)?;
        Ok((bytes, vec![false; req.num_tag]))
    }

    /// Spiegel von `.memWrite` (`ArchSemSequential.lean:65-69`).
    pub fn mem_write(
        &mut self,
        req: &MemRequest<impl Clone, impl Clone>,
        addr: u64,
        value: &[u8],
    ) -> Result<(), SeqError> {
        if req.num_tag != 0 {
            return Err(SeqError::Assertion("Tagged memory not supported".to_string()));
        }
        debug_assert_eq!(value.len(), req.size, "memWrite value must be `size` bytes");
        self.write_bytes(addr, value);
        Ok(())
    }
}

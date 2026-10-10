#![forbid(unsafe_code)]
//! Differentialtests gegen den Lean-Master (offline transkribierte
//! Referenzvektoren; Quellen jeweils am Test). Jede Abweichung = Fehler,
//! Drift blockiert den Merge (E1).

use std::collections::HashMap;

use crate::instantiate::{V1Params, V2Instance};
use crate::v1::{
    AccessKind, AccessStrength, AccessVariety, ExplicitAccessKind, SequentialState as V1State,
};
use crate::v2::{
    Arch as ArchV2, InstructionEffect, MemRequest, RetDesc, SailMemRequest, SeqError,
    SequentialState as V2State,
};

// --- Test-Arch (V2) ---------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TinyAcc {
    Plain,
    Relaxed,
    AcqRcpc,
    AcqRcsc,
    Exclusive,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TinyArch;

impl ArchV2 for TinyArch {
    const ADDR_SIZE: usize = 64;
    const CHERI: bool = false;
    const CAP_SIZE_LOG: usize = 0;
    type AddrSpace = ();
    type Register = String;
    type RegisterValue = u64;
    type MemAcc = TinyAcc;
    type TransStart = ();
    type TransEnd = ();
    type Abort = String;
    type Barrier = ();
    type CacheOp = ();
    type Tlbi = ();
    type Exn = ();
    type SysRegId = ();
    fn mem_acc_is_explicit(a: &TinyAcc) -> bool {
        !matches!(a, TinyAcc::AcqRcpc | TinyAcc::AcqRcsc)
    }
    fn mem_acc_is_ifetch(_: &TinyAcc) -> bool {
        false
    }
    fn mem_acc_is_ttw(_: &TinyAcc) -> bool {
        false
    }
    fn mem_acc_is_relaxed(a: &TinyAcc) -> bool {
        matches!(a, TinyAcc::Relaxed)
    }
    fn mem_acc_is_rel_acq_rcpc(a: &TinyAcc) -> bool {
        matches!(a, TinyAcc::AcqRcpc)
    }
    fn mem_acc_is_rel_acq_rcsc(a: &TinyAcc) -> bool {
        matches!(a, TinyAcc::AcqRcsc)
    }
    fn mem_acc_is_standalone(_: &TinyAcc) -> bool {
        false
    }
    fn mem_acc_is_exclusive(a: &TinyAcc) -> bool {
        matches!(a, TinyAcc::Exclusive)
    }
    fn mem_acc_is_atomic_rmw(_: &TinyAcc) -> bool {
        false
    }
}

type TinyEff = InstructionEffect<
    TinyAcc,
    (),
    String,
    u64,
    (),
    (),
    (),
    (),
    (),
    (),
    (),
>;

// Master: `mem_acc_is_rel_acq := rcpc || rcsc` (ArchSem.lean:36-37).
#[test]
fn v2_rel_acq_is_disjunction_of_rcpc_rcsc() {
    for acc in [
        TinyAcc::Plain,
        TinyAcc::Relaxed,
        TinyAcc::AcqRcpc,
        TinyAcc::AcqRcsc,
        TinyAcc::Exclusive,
    ] {
        let expected =
            TinyArch::mem_acc_is_rel_acq_rcpc(&acc) || TinyArch::mem_acc_is_rel_acq_rcsc(&acc);
        assert_eq!(TinyArch::mem_acc_is_rel_acq(&acc), expected, "rel_acq({acc:?})");
    }
    assert!(TinyArch::mem_acc_is_rel_acq(&TinyAcc::AcqRcpc));
    assert!(TinyArch::mem_acc_is_rel_acq(&TinyAcc::AcqRcsc));
    assert!(!TinyArch::mem_acc_is_rel_acq(&TinyAcc::Plain));
    assert!(!TinyArch::mem_acc_is_rel_acq(&TinyAcc::Relaxed));
}

// Master: `Mem_request.toArchSem` (ArchSem.lean:89-97).
#[test]
fn v2_mem_request_to_archsem_copies_fields() {
    let sail = SailMemRequest {
        size: 4,
        num_tag: 0,
        addr_size: 64,
        access_kind: TinyAcc::Plain,
        address: 0x1000,
        address_space: (),
        size_field: 4,
        num_tag_field: 0,
    };
    let arch: MemRequest<TinyAcc, ()> = sail.to_archsem();
    assert_eq!(arch.access_kind, TinyAcc::Plain);
    assert_eq!(arch.address, 0x1000);
    assert_eq!(arch.size, 4);
    assert_eq!(arch.num_tag, 0);
}

// Master: `inductive InstructionEffect` (ArchSem.lean:157-172) — 15 Ktors.
#[test]
fn v2_instruction_effect_has_all_15_lean_ctors() {
    let mk_req = || MemRequest {
        access_kind: TinyAcc::Plain,
        address: 0,
        address_space: (),
        size: 1,
        num_tag: 0,
    };
    let effs: Vec<(TinyEff, &str)> = vec![
        (TinyEff::RegRead { reg: "r".to_string(), access: None }, "regRead"),
        (
            TinyEff::RegWrite { reg: "r".to_string(), access: None, value: 0 },
            "regWrite",
        ),
        (TinyEff::MemRead(mk_req()), "memRead"),
        (
            TinyEff::MemWrite { req: mk_req(), value: vec![0], tags: vec![] },
            "memWrite",
        ),
        (TinyEff::MemWriteAnnounce(mk_req()), "memWriteAnnounce"),
        (TinyEff::Barrier(()), "barrier"),
        (TinyEff::CacheOp(()), "cacheOp"),
        (TinyEff::TlbOp(()), "tlbOp"),
        (TinyEff::ClockCycle, "clockCycle"),
        (TinyEff::GetCycleCount, "getCycleCount"),
        (TinyEff::TranslationStart(()), "translationStart"),
        (TinyEff::TranslationEnd(()), "translationEnd"),
        (TinyEff::ArchException(()), "archException"),
        (TinyEff::ReturnException, "returnException"),
        (TinyEff::PrintMessage("x".to_string()), "printMessage"),
    ];
    assert_eq!(effs.len(), 15);
    for (e, name) in &effs {
        assert_eq!(e.lean_ctor(), *name);
    }
}

// Master: `InstructionEffect.ret` (ArchSem.lean:174-189).
#[test]
fn v2_ret_mapping_matches_master() {
    let mk_req = |size, num_tag| MemRequest {
        access_kind: TinyAcc::Plain,
        address: 0,
        address_space: (),
        size,
        num_tag,
    };
    assert_eq!(
        TinyEff::RegRead { reg: String::new(), access: None }.ret_desc(),
        RetDesc::RegisterValue
    );
    assert_eq!(
        TinyEff::MemRead(mk_req(4, 0)).ret_desc(),
        RetDesc::MemReadResult { size: 4, num_tag: 0 }
    );
    assert_eq!(
        TinyEff::MemWrite { req: mk_req(4, 0), value: vec![0; 4], tags: vec![] }.ret_desc(),
        RetDesc::MemWriteResult
    );
    assert_eq!(TinyEff::GetCycleCount.ret_desc(), RetDesc::Nat);
    for e in [
        TinyEff::RegWrite { reg: String::new(), access: None, value: 0 },
        TinyEff::MemWriteAnnounce(mk_req(1, 0)),
        TinyEff::Barrier(()),
        TinyEff::CacheOp(()),
        TinyEff::TlbOp(()),
        TinyEff::ClockCycle,
        TinyEff::TranslationStart(()),
        TinyEff::TranslationEnd(()),
        TinyEff::ArchException(()),
        TinyEff::ReturnException,
        TinyEff::PrintMessage(String::new()),
    ] {
        assert_eq!(e.ret_desc(), RetDesc::Unit, "{}", e.lean_ctor());
    }
}

// Master: `writeBytes`/`readBytes` little-endian (ArchSemSequential.lean:30-50).
#[test]
fn v2_sequential_memory_roundtrip_is_little_endian() {
    let mut s: V2State<String, u64> = V2State::new();
    s.write_bytes(0x100, &[0xDE, 0xAD, 0xBE, 0xEF]);
    assert_eq!(s.read_bytes(4, 0x100).unwrap(), vec![0xDE, 0xAD, 0xBE, 0xEF]);
    // Byte an `addr` ist LSB.
    assert_eq!(s.read_byte(0x100).unwrap(), 0xDE);
    assert_eq!(s.read_byte(0x103).unwrap(), 0xEF);
    // Größe 0 liest leer (Master match-arm `0 => pure default`).
    assert_eq!(s.read_bytes(0, 0x500).unwrap(), Vec::<u8>::new());
    // Fehlende Adresse → OutOfMemoryRange (Master `readByte`).
    assert_eq!(s.read_bytes(1, 0x9999).unwrap_err(), SeqError::OutOfMemoryRange(0x9999));
}

// Master: `.memRead`/`.memWrite` lehnen Tags ab (ArchSemSequential.lean:60/66).
#[test]
fn v2_sequential_tagged_memory_rejected_like_master() {
    let s: V2State<String, u64> = V2State::new();
    let tagged = MemRequest {
        access_kind: TinyAcc::Plain,
        address: 0,
        address_space: (),
        size: 1,
        num_tag: 1,
    };
    assert_eq!(
        s.mem_read(&tagged, 0).unwrap_err(),
        SeqError::Assertion("Tagged memory not supported".to_string())
    );
    let mut s = s;
    assert!(s.mem_write(&tagged, 0, &[0]).is_err());
    // Untagged Roundtrip.
    let plain = MemRequest { num_tag: 0, ..tagged };
    s.mem_write(&plain, 0x20, &[0xAB]).unwrap();
    let (bytes, tags) = s.mem_read(&plain, 0x20).unwrap();
    assert_eq!(bytes, vec![0xAB]);
    assert!(tags.is_empty());
}

// Master: V1 `Access_kind`-Konstruktoren (ConcurrencyInterfaceV1.lean:62-70).
#[test]
fn v1_access_kind_ctors_match_master() {
    let e: AccessKind<()> = AccessKind::Explicit(ExplicitAccessKind {
        variety: AccessVariety::Plain,
        strength: AccessStrength::Normal,
    });
    assert_eq!(e.lean_ctor(), "AK_explicit");
    assert_eq!(AccessKind::<()>::Ifetch.lean_ctor(), "AK_ifetch");
    assert_eq!(AccessKind::<()>::Ttw.lean_ctor(), "AK_ttw");
    assert_eq!(AccessKind::Arch(()).lean_ctor(), "AK_arch");
    assert_eq!(AccessVariety::Plain.lean_ctor(), "AV_plain");
    assert_eq!(AccessVariety::Exclusive.lean_ctor(), "AV_exclusive");
    assert_eq!(AccessVariety::AtomicRmw.lean_ctor(), "AV_atomic_rmw");
    assert_eq!(AccessStrength::Normal.lean_ctor(), "AS_normal");
    assert_eq!(AccessStrength::RelOrAcq.lean_ctor(), "AS_rel_or_acq");
    assert_eq!(AccessStrength::AcqRcpc.lean_ctor(), "AS_acq_rcpc");
}

// Master: V1 `sail_mem_write` gibt `Ok (some b)` zurück, `none` schreibt nichts
// (ConcurrencyInterfaceV1.lean:251-256); `readBytes` size 0 → leer (:223-226).
#[test]
fn v1_sequential_mem_roundtrip_matches_master() {
    let mut s: V1State<String, u64> = V1State::new();
    assert!(s.mem_write(0x40, Some(&[0x11, 0x22])));
    assert!(s.mem_write(0x50, None));
    let (bytes, tag) = s.mem_read(2, 0x40).unwrap();
    assert_eq!(bytes, vec![0x11, 0x22]);
    assert_eq!(tag, None);
    assert_eq!(s.mem_read(0, 0x60).unwrap().0, Vec::<u8>::new());
    // No-op-Arme + Zyklenzähler (Master :265-290).
    s.barrier();
    s.cache_op();
    s.tlbi();
    s.translation_start();
    s.translation_end();
    s.take_exception();
    s.return_exception();
    assert_eq!(s.get_cycle_count(), 0);
    s.cycle_count();
    s.cycle_count();
    assert_eq!(s.get_cycle_count(), 2);
}

// Master: `doc_instantiations_v2` (pretty_print_lean.ml:1555-1600).
#[test]
fn v2_instantiation_emitter_matches_master_shape() {
    let inst = V2Instance::new(
        vec![("addr_size".to_string(), "32".to_string())],
        vec![("mem_acc_is_explicit".to_string(), "MyAcc.isExplicit".to_string())],
    );
    let out = inst.emit();
    assert!(out.starts_with("@[reducible]\ninstance : Arch where\n"));
    assert!(out.contains("  addr_size := 32\n"));
    // Defaults aus dem Master (`ts x d` / `is`).
    assert!(out.contains("  addr_space := Unit\n"));
    assert!(out.contains("  CHERI := false\n"));
    assert!(out.contains("  cap_size_log := 0\n"));
    assert!(out.contains("  register := Register\n"));
    assert!(out.contains("  register_type := RegisterType\n"));
    assert!(out.contains("  mem_acc_is_explicit := MyAcc.isExplicit\n"));
    assert!(out.contains("  mem_acc_is_ifetch := fun _ => false\n"));
    assert!(out.contains("  mem_acc_is_atomic_rmw := fun _ => false\n"));
    assert!(out.contains("  trans_start := Unit\n"));
    assert!(out.contains("  sys_reg_id := Unit\n"));
    // Alle 22 Felder: 6 Typen vor mem_acc-Familie, mem_acc + 9 Prädikate, 8 Typen danach.
    assert_eq!(out.lines().filter(|l| l.starts_with("  ")).count(), 6 + 1 + 9 + 8);
}

// Master: `doc_instantiations_v1` (pretty_print_lean.ml:1524-1553).
#[test]
fn v1_instantiation_emitter_matches_master_shape() {
    let p = V1Params {
        pa: "PA".to_string(),
        abort: "Abort".to_string(),
        translation: "Trans".to_string(),
        trans_start: "Ts".to_string(),
        trans_end: "Te".to_string(),
        fault: "Fault".to_string(),
        tlb_op: "Tlbi".to_string(),
        cache_op: "CacheOp".to_string(),
        barrier: "Barrier".to_string(),
        arch_ak: "ArchAK".to_string(),
        sys_reg_id: "SysReg".to_string(),
    };
    let out = p.emit();
    assert!(out.starts_with("instance : Arch where\n"));
    assert!(out.contains("  va_size := 64\n"));
    for line in [
        "  pa := PA\n",
        "  abort := Abort\n",
        "  translation := Trans\n",
        "  trans_start := Ts\n",
        "  trans_end := Te\n",
        "  fault := Fault\n",
        "  tlb_op := Tlbi\n",
        "  cache_op := CacheOp\n",
        "  barrier := Barrier\n",
        "  arch_ak := ArchAK\n",
        "  sys_reg_id := SysReg\n",
    ] {
        assert!(out.contains(line), "missing {line:?}");
    }
}

// Lane-29-Done: `tiny-arm` Arch-Instanz baut.
#[test]
fn tiny_arm_instance_builds() {
    let out = V2Instance::tiny_arm().emit();
    assert!(out.contains("  addr_size := 64\n"));
    assert!(out.contains("  register := TinyReg\n"));
    assert!(out.contains("  register_type := TinyRegType\n"));
    assert!(out.contains("  mem_acc := TinyMemAcc\n"));
    assert!(out.contains("  mem_acc_is_explicit := TinyMemAcc.isExplicit\n"));
    assert!(out.contains("  mem_acc_is_ifetch := fun _ => false\n"));
    assert!(out.contains("  barrier := TinyBarrier\n"));
}

// Adress-Maskierung: `BitVec addr_size`-Trunkierung.
#[test]
fn mem_request_address_masking() {
    let req = MemRequest {
        access_kind: TinyAcc::Plain,
        address: 0x1_FFFF_FFFFu128,
        address_space: (),
        size: 1,
        num_tag: 0,
    };
    assert_eq!(req.masked_address(32), 0xFFFF_FFFF);
    assert_eq!(req.masked_address(64), 0x1_FFFF_FFFF);
    assert_eq!(req.masked_address(0), 0);
    let regs: HashMap<String, u64> = HashMap::new();
    assert!(regs.is_empty());
}

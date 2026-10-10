# SURVEY — Lane 54: Gabbros arm/Arm/Mem-Track

Stand: 2026-10-10. Read-only-Survey (E3: keine Builds/Tests).
Quellen (nur gelesen, nie beschrieben):
- `/home/simon/Dokumente/gabbro-arm/src/lnsym/LNSym/Arm/` (LNSym/Arm/Memory-Track)
- `/home/simon/Dokumente/gabbro-arm/src/sail-arm/` (Sail-ARM-Modelle v8.5/v9.3/v9.4)
- `/home/simon/Dokumente/Sail/lean-sail/Sail/` (Lean-Master: ArchSem)

E1 gilt (Lean = Master, Rust = Spiegel). E1-Einordnung je Datei in der letzten
Tabellenspalte:
- **Spiegel**: Rust muss genau das spiegeln (Master-Referenz angegeben).
- **Referenz**: Beweis-/Modell-Referenz, kein Spiegel (kein zweites Modell).
- **Kein Spiegel**: Gabbro-interne Automatik/Artefakte, nicht spiegeln.

## 1. Mem-Track: `LNSym/Arm/Memory/` (8 Lean-Dateien, 3980 Zeilen)

Basis: `G := /home/simon/Dokumente/gabbro-arm/src/lnsym/LNSym/Arm/Memory/`.

| Pfad (ab `LNSym/Arm/`) | Zeilen | Inhaltstyp | E1-Einordnung |
|---|---|---|---|
| `Memory/Separate.lean` | 709 | Definitionen: `mem_overlap`, `mem_separate`, `mem_subset`, `mem_legal` (Intervall `[a1,a2]` als `BitVec 64`-Paar) + Nat-Längen-Varianten `mem_separate'`/`mem_subset'`/`mem_legal'`, Region-Abstraktion, `read_bytes`/`write_bytes`-Interferenz-Theoreme | Referenz — Separations-Prädikate existieren NUR in Gabbro, kein Gegenstück in lean-sail ArchSem; Lane 29 darf daraus kein zweites Modell ableiten |
| `Memory/SeparateProofs.lean` | 317 | Bewiesene Lemmas zu `Separate` (`mem_subset_refl/trans/eq`, `mem_separate_commutative/neq`, `…_contiguous_regions`, `…_for_subset1/2/general`); Taktik `bvdecide` + LRAT-Zertifikate (s. §3) | Referenz — Beweis-Referenz, kein Spiegel |
| `Memory/MemoryProofs.lean` | 1003 | Read/Write-(Non-)Interferenz: `read_write_same/different`, `read_bytes_write_bytes_same/different`, `write_…_commute/shadow/irrelevant`, `…_subset`-Familie (Autor: Shilpi Goel) | Referenz — Eigenschafts-Referenz für Lane-29-Differentialtests, kein Spiegel |
| `Memory/Common.lean` | 822 | Beweis-Automatik-Infra: `MemSpanExpr`, `MemLegal/Subset/SeparateProp`, `ReadBytes/WriteBytesExpr`, `Hypothesis`, Omega-Anbindung | Kein Spiegel — Gabbro-interne Taktik-Infra |
| `Memory/MemOmega.lean` | 271 | Taktik `mem_omega`/`mem_omega!` (Memory-Seitenbedingungen via Omega) | Kein Spiegel — Gabbro-interne Taktik-Infra |
| `Memory/SeparateAutomation.lean` | 623 | Taktik `simp_mem` (simplifiziert + entlädt Memory-Non-Interference-Seitenbedingungen) | Kein Spiegel — Gabbro-interne Taktik-Infra |
| `Memory/AddressNormalization.lean` | 207 | Simprocs: Bitvektor-Normalisierung (Konstanten links, Assoziativität) für Adress-Ausdrücke | Kein Spiegel — Gabbro-interne Taktik-Infra |
| `Memory/Attr.lean` | 28 | Simp-Attribute (`memory_omega`, `minimal_theory`-nah) + Trace-Klassen (`simp_mem`, `address_normalization`) | Kein Spiegel — Attribut-Deklarationen |

## 2. Arm-Anbindung: State / Map / MinTheory / SMT (4 Dateien, 1617 Zeilen + 73 SMT)

Basis: `G := /home/simon/Dokumente/gabbro-arm/src/lnsym/LNSym/Arm/`.

| Pfad (ab `LNSym/Arm/`) | Zeilen | Inhaltstyp | E1-Einordnung |
|---|---|---|---|
| `State.lean` | 1293 | Zustandsmodell: `Memory := Store (BitVec 64) (BitVec 8)` (Z.134), `ArmState.mem` (Z.147), `read_mem`/`write_mem`, `read_mem_bytes`/`write_mem_bytes` (Z.678–708, Little-Endian `rest ++ byte`), `Memory.read_bytes`/`write_bytes` (Z.860/993), RoW/WoW-Theoreme | Referenz — sequentielles Speichermodell als Vergleichsmaßstab für `SequentialState`; NICHT als Master (Master = lean-sail `ArchSemSequential`) |
| `Map.lean` | 198 | Listen-basierte `Map`-Hilfstyp (Autor: Leonardo de Moura), kein direkter Mem-Bezug | Kein Spiegel — Hilfsdatenstruktur |
| `MinTheory.lean` | 153 | Minimale Simp-Theorie (lineare, seitenbedingungsfreie Lemmas, `minimal_theory`-Attribut) | Kein Spiegel — Taktik-Infra |
| `mem_separate_for_subset.smt2` | 73 | SMT-Extrakt (`separate`/`subset`-Definitionen über `(_ BitVec 64)`, für `mem_separate_for_subset`) | Kein Spiegel — Solver-Artefakt |

## 3. Proof-Zertifikate: `Memory/lrat_files/` (33 Dateien)

Alle `SeparateProofs.lean-*.lrat`: maschinell geprüfte Unerfüllbarkeits-Zertifikate
zu den `bvdecide`-Beweisen in `SeparateProofs.lean` (z. B. `mem_subset_trans-239-2`,
`mem_separate_for_subset2-295/296-2`, je ~13–24 MB bei den großen Transitivitäts-
lemmas). Inhaltstyp: LRAT-Proof-Artefakte. E1: **Kein Spiegel** — Gabbro-interne
Beweisablage, für Rust irrelevant.

## 4. Sail-Seite: `sail-arm` mem/interface (5 Dateien)

Basis: `S := /home/simon/Dokumente/gabbro-arm/src/sail-arm/`.

| Pfad (ab `src/sail-arm/`) | Zeilen | Inhaltstyp | E1-Einordnung |
|---|---|---|---|
| `arm-v9.4-a/src/mem.sail` | 232 | `sail_mem_read`/`sail_mem_write`-Anbindung, exklusive Zugriffe (`Read_exclusive` …), `read_request`/`write_request` | Referenz — Lane-29-`interface.sail`/`mem.sail`-Anbindung; Master bleibt lean-sail, Sail-Datei ist Modell-Input |
| `arm-v9.4-a/src/interface.sail` | 199 | Concurrency-Interface-Instanziierung v9.4 | Referenz — dto. |
| `arm-v9.3-a/src/mem.sail` | 155 | Ältere `mem.sail`-Variante (v9.3, kein `interface.sail` daneben) | Referenz — veraltet, nicht als Basis wählen (Drift-Risiko R3) |
| `arm-v8.5-a/model/aarch_mem.sail` | 11526 | Vollständiges ARMv8.5-Speichermodell (monolithisch) | Referenz — nur für Lane 46 (e2e-armv85) relevant, nicht Lane 29 |
| `arm-v9.{3,4}-a/src/lem-patches/memcpy.sail` | 761 / 803 | `memcpy`-Lem-Patches | Kein Spiegel — Build-Patch, kein Modell |

## 5. Bezug zu lean-sail ArchSem (Lean-Master)

- `lean-sail/Sail/ArchSem.lean` (420 Zeilen): `class Arch` (Z.8–59: `addr_size`,
  `addr_space`, `CHERI`, `register`, `mem_acc`-Familie, `barrier`, `cache_op`,
  `tlbi`, …), `MemRequest` (Z.66+), `InstructionEffect` (Z.157–192).
- `lean-sail/Sail/ArchSemSequential.lean` (105 Zeilen): `SequentialState`
  (`mem : ExtHashMap Nat (BitVec 8)`), `interpretInstructionEffect`
  (`memRead`/`memWrite` sequentiell, `numTag != 0` → Assertion).
- `lean-sail/Sail/ConcurrencyInterfaceV1.lean` (343 Zeilen): V1-Interface.

## 6. Bezug zum Lane-29-Spiegel (`crates/sail-archsem/`)

Der Spiegel ist bereits feld-für-feld dokumentiert (`lib.rs`-Header):
`v2::Arch` ↔ `ArchSem.lean:8-59`, `v2::MemRequest` ↔ `MemRequest`,
`v2::InstructionEffect` ↔ `ArchSem.lean:157-192`, `v2::SequentialState` ↔
`ArchSemSequential.lean:17-79`, `v1` ↔ `ConcurrencyInterfaceV1.lean:27-91`,
`instantiate::emit` ↔ `doc_instantiations_v1/v2`
(`pretty_print_lean.ml:1524-1605`). Gabbro-Dateien kommen darin NICHT vor —
korrekt nach E1.

## 7. Drift-Risiken

- **R1 — Zwei sequentielle Modelle**: Gabbro-`Memory` (`Store (BitVec 64) (BitVec 8)`,
  Adress-Arithmetik in BV64) vs. ArchSemSequential-`mem` (`ExtHashMap Nat (BitVec 8)`,
  `toNat`-Konversion). Byte-Reihenfolge beider `read_bytes` (Append-Richtung) und
  Wrap-around-Semantik bei `addr + 1#64` müssen in Lane-29-Differentialtests
  abgedeckt sein; sonst stiller Drift.
- **R2 — Separations-Prädikate ohne Master-Gegenstück**: `mem_separate/subset/legal`
  nur in Gabbro. Versuchung, sie als Lane-29-Semantik zu übernehmen = zweites
  Modell = E1-Verstoß. Nur als Test-Orakel-Referenz nutzen.
- **R3 — Sail-Versionsdrift**: v9.3-`mem.sail` (155) ≠ v9.4-`mem.sail` (232) +
  `interface.sail` (199); v9.3 hat kein eigenes `interface.sail`. Lane 29 muss
  sich auf EINE Version pinnen (v9.4) und das dokumentieren.
- **R4 — Adressbreite**: Gabbro fix 64-bit; `Arch.addr_size` parametrisch. Der
  Spiegel muss die Parametrisierung erhalten (kein hartkodiertes 64).
- **R5 — Artefakt-Verwechslung**: 33 LRAT-Files + SMT2 sind Gabbro-Beweisablage,
  kein Spiegel-Material. Wer sie portiert, baut ein zweites Modell.

## 8. Fazit

Vermessene Dateien: **12 Lean/SMT-Dateien (5597 Zeilen) + 33 LRAT-Zertifikate
+ 5 Sail-Modelldateien**. Keine Spiegelfläche aus Gabbro nach Rust — alles
Spiegelfläche gegen Lean-Master (§5) bzw. Referenz/Kein-Spiegel (Tabellen).
Lane 29 bleibt korrekt auf lean-sail gepinnt; Gabbro liefert nur das
Test-Orakel-Vokabular (`mem_separate`-Familie, RoW/WoW-Lemmas).

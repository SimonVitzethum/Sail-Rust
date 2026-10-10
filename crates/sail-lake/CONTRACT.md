# Golden Contract — Generated Lean Project Layout

Lane 57 → Lane 45. Static contract for the Lake project skeleton emitted by
`crates/sail-lake` (port of `start_lean_output` / `output_support_lib` /
`create_lake_project` from `sail/src/sail_lean_backend/sail_plugin_lean.ml`,
plus the register/instance templates from
`sail/src/sail_lean_backend/pretty_print_lean.ml`:
`register_enums`, `type_enum`, `inhabit_enum_v1/v2`, `doc_monad_abbrev`,
`doc_instantiations_v1/v2`).

Scope: project skeleton + `Defs.lean` header/register section + root-file
prelude only. Definition bodies, the `end <Camel>.Functions` footer and the
`def main` stub are emitted by the printer/driver stages and are out of
scope for this contract.

E3: this contract requires no builds and no `lake`. Validation is a pure
string comparison against `golden/` (see "Checking" below).

## E2 — no-`sorry` policy

No file under `golden/` contains the token `sorry` (verified with
`grep -r "sorry" crates/sail-lake/golden/`, zero matches). The generator
itself never emits it either: `sail-lake/src/lib.rs` names the token only
in `NO_SORRY_NOTE` / `ARM_EXTRAS_MODULE` docs and in the
`generator_never_emits_sorry` test, which asserts
`!layout_snapshot().contains("sorry")`. `decreasing_trivial` is never
emitted by the generator; it resolves from `Sail.ArmExtras`, which is
imported only when the opt-in `with_arm_extras` option is set (off by
default, byte-compatible with the OCaml backend).

## Pinned constants

| Item | Value | OCaml source |
|---|---|---|
| `lean-toolchain` contents | `leanprover/lean4:v4.29.0`, no trailing newline | `lean_version` |
| lean-sail git URL | `https://github.com/rems-project/lean-sail` | `lib_default_git` |
| lean-sail rev | `v6` | `lib_default_rev` |
| mathlib git URL | `https://github.com/leanprover-community/mathlib4-nightly-testing` | inline in `create_lake_project` |
| mathlib rev | `v4.29.0` | `mathlib_version` |
| lake manifest version | `1.1.0`, `packagesDir` `.lake/packages`, empty `packages`, `lakeDir` `.lake` | `create_lake_project` |
| interface modules | V1 `ConcurrencyInterfaceV1` + `SpecializationV1.lean`, V2 `ArchSem` + `SpecializationArchSem.lean` | `interface_module`, `CONCURRENCY_INTERFACE_V2` |
| module case | `UpperCamelCase(out_name)` (`tiny_arm` → `TinyArm`) | `Libsail.Util.to_upper_camel_case` |

## File layout (`LakeProject::layout`)

| Path | Contents rule |
|---|---|
| `.gitignore` | `/.lake`, no trailing newline |
| `lean-toolchain` | `leanprover/<toolchain_version>`, no trailing newline |
| `lakefile.toml` | `render_lakefile` (see below) |
| `lake-manifest.json` | `render_lake_manifest` (exact 6-line JSON block, trailing newline) |
| `<Camel>/Defs.lean` | `import Sail` + `open PreSail` + file prelude + register section |
| `<Camel>.lean` | function-file prelude (see below; body out of scope) |

Snapshot envelope used by every `golden/layout-*.txt`:
`=== <path> ===\n<contents>\n` per file, in the table order above
(`LakeProject::layout_snapshot`).

## File prelude (every `.lean` file)

```lean
set_option maxHeartbeats 1_000_000_000
set_option maxRecDepth 1_000_000
set_option linter.unusedVariables false
set_option match.ignoreUnusedAlts true

open Sail
open Sail.<ConcurrencyInterfaceV1|ArchSem>

[noncomputable section
]namespace <Camel>

```

`noncomputable section` appears only with `--lean-noncomputable`.
`Defs.lean` prepends `import Sail\nopen PreSail\n\n` before this block.

## Root function-file prelude (`<Camel>.lean`)

- No `import_refs`: `import Sail`, `import <Camel>.Defs`, then one
  `import <Camel>.<extra>` per extra import (copied `--lean-import-file`
  modules such as `FakeReal`, plus the `Specialization*` shim), in order.
- With `import_refs`: one `import <Camel>.<ref>` per ref instead (no
  `import Sail` / `.Defs` lines).
- With `with_arm_extras`: `import Sail.ArmExtras` appended after the above.
- Then blank line, file prelude, `open <interface module>`, blank line,
  `namespace Functions`, blank line.

## `lakefile.toml` blocks (`render_lakefile`)

```toml
name = "<out_name>"
defaultTargets = ["<Camel>"]
moreLeanArgs = ["--tstack=400000"]

[[lean_lib]]
name = "<Camel>"
leanOptions.weak.linter.style.nameCheck = false
moreLeancArgs= ["-fbracket-depth=500"]

[[require]]
name = "Sail"
<git = "<url>" / rev = "<rev>" | path = "<path>">
```

- `[[require]] name = "Sail"`: git+rev by default; `--lean-lib-path`
  replaces both lines with a single `path = "..."` line; git+path together
  is a hard error (`--lean-lib-git and --lean-lib-path cannot be set
  simultaneously`).
- `--lean-real-numbers` appends a `[[require]] name = "mathlib"` block
  (git + `rev = "v4.29.0"`). Without the flag no mathlib block exists.
- `--lean-executable` (only when the model defines `main`) appends
  `[[lean_exe]] name = "run"` with `root = "<Camel>"`. Without the flag no
  `lean_exe` block exists.

## Register / instance section (`render_register_section`)

Empty model (no registers):

```lean
abbrev Register := PEmpty
abbrev RegisterType : Register -> Type := PEmpty.elim

```

Non-empty model:

```lean
inductive Register : Type where
  | <ctor>
  ...
  deriving DecidableEq, Hashable, Repr
open Register

abbrev RegisterType : Register → Type   -- U+2192 arrow, as in OCaml
  | .<ctor> => <lean-type>
  ...

<arch instance block, V1 conditional | V2 always>

<monad abbrevs>

<inhabited instances>
```

- V1 arch block: `instance : Arch where` with only the configured fields,
  in OCaml order (`pa`, `abort`, `translation`, `trans_start`,
  `trans_end`, `fault`, `tlb_op`, `cache_op`, `barrier`, `arch_ak`,
  `sys_reg_id`); omitted entirely when no field is set.
- V2 arch block: always emitted, `@[reducible]\ninstance : Arch where`
  with OCaml defaults (`addr_size 64`, `addr_space Unit`, `CHERI false`,
  `cap_size_log 0`, `register Register`, `register_type RegisterType`,
  `mem_acc Unit`, all nine `mem_acc_*` predicates `fun _ => false`,
  remaining fields `Unit`).
- Monad abbrevs: `abbrev exception := Unit` first, unless the model defines
  its own `exception` type; then
  V1 `abbrev SailM := PreSailM RegisterType trivialChoiceSource exception` /
  `abbrev SailME := PreSailME RegisterType trivialChoiceSource exception`,
  V2 `abbrev SailM := PreSailM exception` /
  `abbrev SailME := PreSailME exception`.
- `Inhabited` instances: one per distinct register Lean type (dedup keeps
  the first constructor per type), `default := .Reg <first-ctor>`;
  V1 wraps the type as `RegisterRef RegisterType <ty>`, V2 as
  `RegisterRef <ty>`.
- Known OCaml delta: OCaml `doc_instantiations_v1` emits a leading
  `va_size := 64` line and emits the V1 arch block only when monad
  parameters exist; the Rust `ArchFieldsV1` port renders exactly the set
  fields with no `va_size` line. The fixtures pin the Rust behavior.

## Fixture manifest (`golden/`)

| Fixture | Generator inputs (`LakeProject`, `out_name = "tiny_arm"`) |
|---|---|
| `layout-default.txt` | defaults: V1, no flags, no imports, no registers |
| `layout-v2-full.txt` | V2 + `noncomputable` + `real_numbers` + `executable`; `extra_imports = [FakeReal, SpecializationArchSem]`; registers `x1/x2 : (BitVec 64)`, `v0 : (BitVec 128)`; default V2 arch; no own `exception` type |
| `layout-v1-regs.txt` | V1 + `lib_path = /opt/lean-sail` + `with_arm_extras`; registers `x1/x2 : (BitVec 64)` (dedup → one `Inhabited` instance); own `exception` type; V1 arch `{pa := (BitVec 64), abort := Unit, fault := (BitVec 64)}` |
| `register-empty.txt` | `render_register_section` of the default project |
| `register-v1.txt` | `render_register_section` of the v1-regs project |
| `register-v2.txt` | `render_register_section` of the v2-full project |

`layout-*.txt` files are full `layout_snapshot()` outputs; `register-*.txt`
files are the standalone `Defs.lean` register sections for direct
comparison.

## Checking (Lane 45)

1. `grep -r "sorry" crates/sail-lake/golden/` → zero matches (E2).
2. Build the same three `LakeProject` inputs from the manifest, render via
   `layout_snapshot()` / `render_register_section()`, byte-compare against
   the fixtures (exact bytes, including trailing newlines and the two blank
   lines after a V1 arch block). No `cargo`/`lake` invocation needed for
   the check itself.
3. Byte-exactness anchor: the `small_project()` unit case in
   `sail-lake/src/lib.rs` (`test_model`, V2, same registers/imports as
   `layout-v2-full.txt` modulo `out_name` and flags) is pinned to FNV-1a
   `0548491546d4278b`. The `golden/` fixtures were generated from a
   line-for-line emulation of the render functions that reproduces this
   hash exactly.

Refresh rule: fixtures change only deliberately with the template change,
never blindly; keep this document and the manifest in sync.

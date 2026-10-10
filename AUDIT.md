# Lane 67 — Supply-Chain-Audit (Report)

Stand: 2026-10-10 · Methode: lesend, **keine Builds/Tests (E3)**.
Quellen: `crates/*/Cargo.toml` (direkte Deps), RustSec Advisory-DB per
Netz-Abfrage (GitHub-API auf `rustsec/advisory-db`, `main`-Branch).
Config: `deny.toml` (Workspace-Root).

Einschraenkung: Das Repo checkt **kein `Cargo.lock`** ein. Transitive
Versionen konnten daher nicht aufgeloest werden; der Advisory-Status unten
gilt pro direkter Abhaengigkeit gegen die aktuelle DB. Der erste echte
`cargo deny check` (nach Aktion A1) ersetzt diesen Report als Quelle.

## 1. Advisory-Ergebnis (direkte externe Deps)

| Dep (Requirement) | Verwendet in | Advisories in DB | Bewertung |
|---|---|---|---|
| `rayon` 1 / 1.10 | sail-rewrite | keine | ok |
| `blake3` 1 | sail-shard | keine | ok |
| `lalrpop` / `lalrpop-util` 0.22 | sail-grammar | keine | ok |
| `proptest` 1 (dev) | sail-nexp | keine | ok |
| `serde` 1 + `serde_json` 1 | sail-ast, sail-perf u.a. | keine | ok |
| `crossbeam-channel` 0.5 | sail-shard | 3 (s. u.) | **Aktion A4** |
| `z3` 0.12 (optional) | sail-solver | **RUSTSEC-2026-0295** | **Aktion A5** |

Details:

- `crossbeam-channel`: RUSTSEC-2020-0052 (gepatcht ≥ 0.4.4) und
  RUSTSEC-2022-0019 (`informational = "unsound"`, gepatcht ≥ 0.4.3)
  betreffen nur 0.4.x — fuer Requirement `0.5` irrelevant.
  **RUSTSEC-2025-0024 (CVE-2025-4574, Double-Free-Race, betrifft
  > 0.5.11 und < 0.5.15, gepatcht ≥ 0.5.15)** liegt im Aufloesebereich
  von `0.5`. Eine frische Aufloesung zieht ≥ 0.5.15 (Fix seit 2025-04),
  aber ohne Lockfile ist das weder garantiert noch pruefbar.
- `z3` 0.12: **RUSTSEC-2026-0295 (2026-08-12, Memory-Corruption /
  Use-after-free in `ApplyResult::clone`, gepatcht ≥ 0.21.0)**.
  Requirement `0.12` < 0.21.0 → bei aktiviertem Feature betroffen.
  Einziger echter Vuln-Treffer dieses Audits.

## 2. Lizenz-Ergebnis (Policy: MIT/Apache-2.0-only)

> Nachtrag Lane 113: Die Abweichung unten ist erledigt — alle
> Workspace-Crates deklarieren `MIT OR Apache-2.0` (s. § 5).

- Workspace (`Cargo.toml`): `license = "MIT OR Apache-2.0"` — ok.
- Externe Deps: alle MIT und/oder Apache-2.0 (blake3: CC0-1.0 OR
  Apache-2.0 → ok ueber Apache-2.0-Alternative). Endgueltig verifiziert
  das erst `cargo deny check licenses` nach A1.
- **Abweichung Workspace-Crates** (faellt unter `deny.toml [licenses]`
  durch, bis A2/A3 umgesetzt):
  - `BSD-2-Clause` (5): sail-extern, sail-grammar, sail-mapping,
    sail-parse, sail-prelude.
  - **kein `license`-Feld** (6): sail-archsem, sail-int, sail-lean,
    sail-nexp, sail-patterns, sail-regs (`unlicensed = "deny"` greift).
  - Rest: `MIT OR Apache-2.0` bzw. `license.workspace = true` — ok.

## 3. Aktionen (Folgearbeit, nicht Teil dieser Lane)

- A1: `Cargo.lock` einchecken (oder CI-Job mit `cargo generate-lockfile`
  davor) — Voraussetzung fuer reproduzierbare `cargo deny`-Laeufe.
- A2: 5 BSD-2-Clause-Crates auf `license.workspace = true` umstellen
  (falls Policy MIT/Apache-2.0-only Bestand hat; sonst Policy aendern).
- A3: 6 Crates ohne `license`-Feld auf `license.workspace = true` setzen.
- A4: `crossbeam-channel`-Requirement auf `0.5.15` anheben (Pin unter
  die gepatchte Version unmoeglich machen).
- A5: `z3` 0.12 → ≥ 0.21 anheben **oder** dokumentieren, dass das
  Solver-Feature bis dahin nicht aktiviert werden darf.
- A6: CI-Job nach Vorschlag unten einbauen (Lane 91: umgesetzt als
  warnender `deny-audit`-Job mit `continue-on-error: true`).

## 4. CI-Einbau (Lane 91: warnend verdrahtet)

Verdrahteter Job in `.github/workflows/ci.yml` (vorerst warnend):

```yaml
  deny-audit:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: cargo-deny installieren
        run: cargo install cargo-deny --locked
      # Entfaellt, sobald Cargo.lock eingecheckt ist (Aktion A1).
      - name: Lockfile aufloesen (lesend, kein Build)
        run: cargo generate-lockfile
      - name: cargo deny (Advisories + Lizenzen + Bans + Sources)
        run: cargo deny check advisories licenses bans sources
```

Reihenfolge-Hinweis: Erst A1–A5 umsetzen, dann Job scharf schalten
(`continue-on-error: true` in `.github/workflows/ci.yml` entfernen) —
sonst faellt er auf den hier dokumentierten Abweichungen (Lizenzen,
ggf. z3) rot aus. Der Job laeuft bewusst vorerst warnend (analog
`perf-budget`).
`cargo-deny`-Version dabei per `--locked` bzw. fester Versionsnummer
pinnen, damit DB-Format (`[advisories] version = 2`,
`[licenses] version = 2`) stabil bleibt.

## 5. Nachtrag Lane 113 (Stand 2026-10-10, lesend, E3 — kein Build/Lauf)

| Aktion | Status | Beleg (Review, ohne Build) |
|---|---|---|
| A1 Lockfile | **offen, Blocker E3** | Kein `Cargo.lock` im Repo; `cargo generate-lockfile` ist ein Lauf und damit ausserhalb E3. CI-`deny-audit` bleibt warnend. |
| A2 BSD-Lizenzen | **geschlossen** | Kein `BSD-` mehr in `crates/*/Cargo.toml`; alle 5 genannten Crates heute `MIT OR Apache-2.0` (Umstellung aus Lane 70). |
| A3 license-Felder | **geschlossen** | Jedes Workspace-Member hat ein `license`-Feld (direkt oder `license.workspace = true`); Luecke per Schleife ueber alle Member geprueft. |
| A4 crossbeam-channel | **geschlossen** | `sail-shard` (Lane 70) und `sail-par-pipe` (diese Lane: `"0.5"` → `"0.5.15"`) fordern ≥ 0.5.15 — CVE-2025-4574-Bereich (`> 0.5.11, < 0.5.15`) ist per Requirement ausgeschlossen. |
| A5 z3 | **geschlossen (Code) / Lauf nach A1** | `sail-solver`: `z3 = "0.21"` (≥ 0.21.0, Fix von RUSTSEC-2026-0295). Review von `z3_backend.rs`: 0.21-API ohne `Context`-Handles (`Solver::new()`, `Params::set_u32("timeout", …)`, `new_const`/`fresh_const`), kein `ApplyResult`-Gebrauch (Vuln-Muster). Feature weiter optional/default-aus. Endgueltig bestaetigt das erst `cargo deny check` nach A1. |
| geiger-Doku | **geschlossen (Doku)** | Eigener Code: `forbid(unsafe_code)` in jedem Crate plus Workspace-Lint (`[workspace.lints]`, nicht aufweichbar); `unsafe`-Token-Grep- und `cargo geiger --forbid-only`-Gate in CI (`unsafe-verbot`-Job). `unsafe` steckt nur in Fremd-Deps (z3-C-FFI, siehe `z3_backend.rs`-Modul-Doku); Ausnahmepfad dokumentiert in `docs/branch-protection.md`. Lauf (`cargo geiger`) braucht Aufloesung und folgt nach A1. |

Schaerfung (warnend → scharf): In `deny.toml` bereits maximal, wo per
Review belegbar (`vulnerability`/`yanked`/`unlicensed`/`wildcards`/
`unknown-*` = `deny`). `unmaintained`/`notice`/`multiple-versions`
bleiben warnend (Begruendung in `deny.toml`); `deny-audit`-Job bleibt
warnend bis A1.

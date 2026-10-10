# WAVE3 — Merge-Plan dritte Welle (Integrator-Anleitung, blind abarbeitbar)

Stand: 10.10.2026, erhoben read-only aus Worktree `lane-56`
(Branch `lane/56-wave3-prep`). E3 gilt weiter: **kein `cargo`/Build/Test
durch Lane-Agents** — Verifikation läuft ausschließlich zentral beim
Integrator, sequenziell, mit Memory-Limits. Nichts in diesem Plan wurde
gemergt oder gebaut; alle Aussagen per `git merge-base` / `git diff` erhoben.

## 0. Lese-Reihenfolge (vor dem ersten Merge)

1. `UNIFY_PLAN.md` (Lane 48, Wurzeldatei) — Ist-Analyse + behalten/verwerfen-Regeln.
2. Diese Datei von oben nach unten.

## 1. Branch-Befund (Basis → eigene Commits, `merge-base..branch`)

| Branch (Lane) | Eigene Commits ab Basis | Enthält | Mergebar? |
|---|---|---|---|
| `lane/11-sharding-incremental` (11) | in main gemergt (`7a35781`) | — | ERLEDIGT, kein Schritt |
| `lane/structs-enums-unions` (30) | **keine** (HEAD == Basis `8da4efe`) | — | OFFEN, aber leer → kein Merge-Schritt |
| `lane/41-lang-delta` (41) | 1 (`4d6e1e0`) | `docs/multicore/SYNTAX.md` + 3 Snippets | **BLOCKED (Gate, §2)** |
| `lane/42-shared-mem` (42) | **keine** (HEAD == Basis `874a24c`) | — | leer → kein Merge-Schritt |
| `lane/43-scheduler-replay` (43) | **keine** (HEAD == Basis `874a24c`) | — | leer → kein Merge-Schritt |
| `lane/44-el3-psci-boot` (44) | eigene Commits | `crates/sail-psci-boot/` (neu) + 1 Member-Zeile in Root-`Cargo.toml` | **BLOCKED (Gate, §2)** |
| `lane/45-e2e-tiny-arm` (45) | 1 (`5dbe27d`) | `crates/sail-cli/src/e2e.rs` (neu), `main.rs`, `Cargo.toml` (sail-cli), `lanes/45-e2e-tiny-arm-fails.md` | JA, mit Konflikt (§4.2) |
| `lane/46-e2e-armv85` (46) | 1 (`488c49f`) | `perf/arm-v8.5-a-plan.md`, `perf/measure-armv85.sh` | JA, konfliktfrei |
| `lane/47-ast-unify` (47) | **keine** | — | leer → kein Merge-Schritt (Implementierung steht aus) |
| `lane/48-workspace-unify` (48) | 1 (`e15e7d1`) | `UNIFY_PLAN.md` (Wurzel, Analyse only) | JA, konfliktfrei (§4.1 zuerst) |
| `lane/49-ci-gates` (49) | 1 (`192efa3`) | `.github/workflows/ci.yml`, `docs/branch-protection.md` | JA, ggf. Kleinkonflikt (§4.3) |
| `lane/50-smp-boot` (50) | 1 (`3212a72`) | `docs/smp-boot-acceptance.md`, `scripts/smp-boot-acceptance.sh` | JA, konfliktfrei |
| `lane/51-test-rig` (51) | 1 (`1cb66b0`) | `.gitignore` (+`logs/`), Root-`Cargo.toml` (`[profile.ci]`-Anhang), `docs/TEST-RIG.md`, `lanes/51-test-rig.md`, `scripts/limited-run.sh`, `scripts/tmux-test.sh` | JA, ggf. Kleinkonflikt (§4.4) |
| `lane/52-sorry-elim` (52) | **keine** (HEAD == Basis `4bc834a`) | — | leer → kein Merge-Schritt |
| `lane/53-arch-docs` (53) | **keine** | — | leer → kein Merge-Schritt |
| `lane/54-gabbro-survey` (54) | **keine** | — | leer → kein Merge-Schritt |
| `lane/55-budget-consol` (55) | **keine** | — | leer → kein Merge-Schritt |
| `lane/57-lean-contract` (57) | **keine** | — | leer → kein Merge-Schritt |
| `lane/58-ram-opt` (58) | **keine** (HEAD == main-HEAD `b079704`) | — | **BLOCKED (Gate, §2)** |

Basis-Legende: 41–51 (außer 51) sitzen auf `874a24c` (Spec-Commit vor Welle 2);
52–55/57 auf `4bc834a`; 58 auf aktuellem main-HEAD. Main-HEAD ist `b079704`
(Merge Lane 40), Workspace hat dort **35 Member**.

## 2. Gate (BLOCKED — nicht mergen, nicht starten)

Folgende Branches sind bis zur Gate-Freigabe **gesperrt**:

- **Lane 41–44 (Multicore): NICHT mergen**, bis alle drei Bedingungen erfüllt sind:
  1. **Port durch** — Phase-1-Parität: E2E-Pipeline Parse→Lean läuft
     (Lane-45-Fail-Liste abgearbeitet bzw. Fail-Owner bestätigt);
  2. **läuft** — E2E-Durchläufe Tiny-Arm (Lane 45) und arm-v8.5-a (Lane 46)
     sind beim Integrator mit Memory-Limits gelaufen und protokolliert;
  3. **RAM-opt** — Lane-58-RAM-Optimierung umgesetzt und Peak-Budgets
     (Lane 14, `perf/BUDGETS.md`-Format) eingehalten.
- **Lane 58 (ram-opt): startet erst nach Unify** (Lane 47 Ein-AST +
  Lane 48 Workspace-Bereinigung) **und** unterliegt demselben Gate —
  vorher kein Merge, vorher kein Start.
- Leere Multicore-Branches (42, 43) sind mangels Inhalt ohnehin kein
  Merge-Schritt; sobald sie implementiert werden, fallen sie unter dasselbe Gate.

Freigabe dokumentiert der Integrator per Commit in `ACTIVE.md`
(Status 41–44 / 58 von QUEUED/BLOCKED auf freigegeben).

## 3. Merge-Reihenfolge (nur nicht-blockierte, nicht-leere Branches)

Reihenfolge-Grundsatz: **Plan zuerst, dann konfliktfreie Docs/Scripts,
dann Cargo-klein, dann Cargo-schwer, CI-Gates zuletzt.**

1. Lane 48 → 2. Lane 46 → 3. Lane 50 → 4. Lane 51 → 5. Lane 45 → 6. Lane 49.
41/44/58 bleiben draußen (Gate); 30/42/43/47/52–55/57 sind leer (kein Schritt).

## 4. Exakte Befehlsfolge

Alle Merges laufen auf frischem Stand, je ein Merge-Commit mit Sign-off.
Platzhalter `<datum>` = Tagesdatum `YYYY-MM-DD`.

```sh
git checkout main
git pull --ff-only
git checkout -B wave3-integration main
```

### 4.1 Schritt 1 — Lane 48 (UNIFY_PLAN, konfliktfrei erwartet)

```sh
git merge --no-ff --no-commit lane/48-workspace-unify
git status --short   # erwartet nur: A UNIFY_PLAN.md
git commit -s -m "Merge lane/48-workspace-unify (Lane 48 UNIFY_PLAN, Analyse only)"
```

Erwartung: genau eine neue Datei `UNIFY_PLAN.md` im Wurzelverzeichnis.
Kein doppeltes Crate, keine `Cargo.toml`-Berührung. Falls doch mehr als diese
eine Datei im Index steht: abbrechen (`git merge --abort`), Befund an Lane 48 melden.

### 4.2 Schritt 2 — Lane 46 (perf-Docs, konfliktfrei erwartet)

```sh
git merge --no-ff --no-commit lane/46-e2e-armv85
git status --short   # erwartet nur: A perf/arm-v8.5-a-plan.md, A perf/measure-armv85.sh
git commit -s -m "Merge lane/46-e2e-armv85 (Lane 46 Durchlaufplan + Messgeruest)"
```

### 4.3 Schritt 3 — Lane 50 (SMP-Abnahme Doku+Skript, konfliktfrei erwartet)

```sh
git merge --no-ff --no-commit lane/50-smp-boot
git status --short   # erwartet nur: A docs/smp-boot-acceptance.md, A scripts/smp-boot-acceptance.sh
git commit -s -m "Merge lane/50-smp-boot (Lane 50 SMP-Boot-Abnahme Doku + Harness-Skizze)"
```

### 4.4 Schritt 4 — Lane 51 (Test-Rig; `.gitignore` + Root-`Cargo.toml`-Anhang)

```sh
git merge --no-ff --no-commit lane/51-test-rig
git status --short
```

Erwartete Konfliktstelle (falls Konflikt): Root-`Cargo.toml` — der Branch hängt
`[profile.ci]` ans Dateiende an. Auflösung: **beide Seiten behalten**
(HEAD-Inhalt vollständig übernehmen, `[profile.ci]`-Block aus der Lane ans Ende
anfügen). `.gitignore` (`logs/`-Zeile) ebenfalls beide Seiten vereinen.
Danach:

```sh
git add -A
git commit -s -m "Merge lane/51-test-rig (Lane 51 Test-Rig + ci-Profil)"
```

### 4.5 Schritt 5 — Lane 45 (sail-cli E2E-Verdrahtung; Konflikt erwartet)

```sh
git merge --no-ff --no-commit lane/45-e2e-tiny-arm
git status --short
```

Erwartete Konflikte (Branch-Basis `874a24c` kennt Lane-38-cli-frontend nicht):

- `crates/sail-cli/src/main.rs` — **HEAD-Seite gewinnt**, Lane-Code wird nicht
  1:1 übernommen. Stattdessen von Hand nachziehen:
  `crates/sail-cli/src/e2e.rs` aus der Lane übernehmen
  (`git checkout lane/45-e2e-tiny-arm -- crates/sail-cli/src/e2e.rs`),
  Modul in HEAD-`main.rs` registrieren (`mod e2e;` + `--e2e`-Verdrahtung),
  `lanes/45-e2e-tiny-arm-fails.md` übernehmen.
- `crates/sail-cli/Cargo.toml` — **HEAD-Seite gewinnt**; nur fehlende Deps
  für `e2e.rs` aus der Lane-Version ergänzen, nichts streichen.
- `Cargo.toml` (Root), `Carbon`-Reste o. Ä. aus der Lane-Seite: **verwerfen**
  (alte 27-Member-Liste der Lane darf die 35-Member-Liste von HEAD nicht ersetzen).

```sh
git add -A
git commit -s -m "Merge lane/45-e2e-tiny-arm (Lane 45 E2E-Skizze Parse->Lean + Fail-Liste)"
```

### 4.6 Schritt 6 — Lane 49 (CI-Gates; zuletzt, ggf. Kleinkonflikt)

```sh
git merge --no-ff --no-commit lane/49-ci-gates
git status --short
```

Konfliktkandidat: `.github/workflows/ci.yml` (Lane-15-Basis vs. seitherige
CI-Änderungen). Auflösung: beide Job-Mengen vereinen — E2-sorry-Gate,
Unsafe-Gate (geiger + Token-Grep) und Budget-Gate (warnend,
`continue-on-error: true`) aus der Lane übernehmen, alle HEAD-Jobs behalten.
`docs/branch-protection.md`: beide Seiten vereinen.

```sh
git add -A
git commit -s -m "Merge lane/49-ci-gates (Lane 49 E2-/Unsafe-/Budget-Gates in CI)"
```

### 4.7 Abschluss (Integrator, sequenziell, mit Memory-Limits)

```sh
git log --oneline -8          # 6 Merge-Commits aus §4.1-§4.6 sichtbar?
git status --short            # sauber?
git checkout main
git merge --ff-only wave3-integration
git branch -d wave3-integration
```

Danach `ACTIVE.md` nachführen (48/46/50/51/45/49 als DONE, Rest BLOCKED/leer),
`git push`. Erst danach (und nur dann) Gate-Prüfung §2 für 41–44/58.

## 5. Doppelte Crates / Cargo.toml-Member (Konflikt-Tabelle)

| Stelle | Befund | Regel |
|---|---|---|
| `crates/sail-psci-boot/` (Lane 44) | Crate existiert in main **nicht** (kein Duplikat), aber Lane ist **BLOCKED** | kein Schritt in Welle 3; bei späterem Merge: Verzeichnis übernehmen + genau eine Member-Zeile `"crates/sail-psci-boot",` in die HEAD-`members`-Liste eintragen; Lane-`Cargo.toml` (alte Member-Liste) **verwerfen** |
| Root-`Cargo.toml` (Lane 44 vs. 51) | beide fassen dieselbe Datei an (44: Member-Mitte, 51: Profil-Anhang) | gegenstandslos in Welle 3 (44 blocked); nur 51 wird gemergt (§4.4) |
| `crates/sail-cli/` (Lane 45) | kein neues Crate, aber `main.rs`/`Cargo.toml` gegen Lane-38-Stand in main veraltet | HEAD gewinnt, `e2e.rs` + Fail-Liste nachziehen (§4.5) |
| `[profile.ci]` (Lane 51) | kein Duplikat (Profil existiert in main nicht) | Anhang übernehmen (§4.4) |
| `.github/workflows/ci.yml` (Lane 49) | kein Crate, Job-Erweiterung auf Lane-15-Basis | Jobs vereinen, nichts streichen (§4.6) |
| `sail-typedef` / `mini_ast`-Duplikate (Lane 30/47) | gehören Lane 47 (Ein-AST), nicht in Welle 3 | Integrator stimmt Lane 47 mit 48-Plan (§1.2 dort) ab |

## 6. Nicht in Welle 3 (Zusammenfassung)

- **BLOCKED (Gate §2):** 41 (docs-only, 4 Dateien), 44 (`sail-psci-boot`),
  58 (leer, startet erst nach Unify).
- **Leer, kein Merge-Schritt:** 30 (`lane/structs-enums-unions`, keine eigenen
  Commits), 42, 43, 47, 52, 53, 54, 55, 57.
- **Bereits erledigt:** 11 (in main gemergt, `7a35781`).

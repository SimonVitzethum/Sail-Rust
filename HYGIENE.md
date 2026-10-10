# HYGIENE.md — Lane 118 Rewrite-Bereitschaft (read-only Befund)

Stand: 2026-10-10 · HEAD `f6e3368` · Methode: nur gelesen (`git log/show/fsck/branch`),
keine History-Umschreibung, keine Builds. Soll-Trailer:
`Signed-off-by: SimonVitzethum <simon.vitzethum@icloud.com>` +
`Co-authored-by: muse-spark-1.3 <muse-spark-1.3@noreply.local>` als
eigener Block am Message-Ende (eine Trailer-Zeile pro Zeile, Blank-Line davor).

## Gesamtbefund main (287 Commits, `rev-list --count main`)

- Mit beiden Trailern (Werte konform): **179**
- Ohne beide Trailer: **108**
- Nur ein Trailer: **0** (paarweises Muster: beide oder keiner)
- Malformiert (Trailer inline im Fließtext, kein Trailer-Block): **1**
  (`ab5ae5d` Lane 78 — `...gelesen. Signed-off-by: ... Co-authored-by: ...` in einer Zeile)
- Identität: **210×** `SimonVitzethum <simon.vitzethum@icloud.com>`,
  **77×** `sail <sail@local>` (alle 77 ohne Trailer, v. a. frühe Lane-Merges).
- `commit.template=.gitmessage` enthält nur die Co-Zeile; `-s` bleibt nötig für Sign.

## Je-Branch-Urteil

| Branch | Spitze | Unique vs main | Trailer unique | Urteil |
|---|---|---|---|---|
| main | `f6e3368` | — | — | **NICHT-BEREIT** (108 ohne Trailer, 77 Fremd-Identität, 1 malformiert) |
| lane/104-grammar-next … lane/111, lane/113–119 (15 Branches) | `f6e3368` (identisch) | 0 | — | **NICHT-BEREIT** (geerbt von main) |
| lane/41-lang-delta | `4d6e1e0` | 1 | beide ok | **NICHT-BEREIT** (Basis 179 hinter main; Gesamthistorie wie main) |
| lane/42-shared-mem | `9a00e10` | 2 | beide ok | **NICHT-BEREIT** (dto.) |
| lane/43-scheduler-replay | `744bbb8` | 1 | beide ok | **NICHT-BEREIT** (dto.) |
| lane/44-el3-psci-boot | `9a61e7d` | 1 | beide ok | **NICHT-BEREIT** (dto.) |

Unique-Commits 41–44 selbst sind konform (korrekte Identität + beide Trailer),
aber kein Branch ist als Ganzes rewrite-bereit: jeder enthält die 108
trailerlosen + 77 `sail@local`-Commits aus main.

## Waisen (44 unreferenzierte Commits, `git fsck`)

- Alle Identität `SimonVitzethum <simon.vitzethum@icloud.com>` (kein Identitäts-Problem).
- Trailer: 38× Sign, 23× Co, 22× beide, **5× weder** — darunter
  3× `WIP on lane/71-payload-intern` (Stash-Artefakte: `eac1dea`, `0465993`, `3cb38a7`),
  1× `WIP on lane/99-grammar-rest` (`2926a5f`), 1× `lane-08: sail-effects` (`05a0917`).
- Mehrere Waisen tragen Trailer **mitten im Body** statt im End-Block
  (z. B. `8784f71`, `a20df36`, `f75bd79`) — Rewrite muss entscheiden: fallen lassen vs. retten.
- Kein Dangling im engeren Sinn; alles `unreferenziert` (Reflog-/Stash-Reste).

## Rewrite-Hemmnisse (für das eigene Rewrite-Kommando)

1. 108 Commits ohne Trailer → Trailer-Insert nötig.
2. 77 Commits Fremd-Identität `sail <sail@local>` → Identitäts-Normalisierung nötig.
3. 1 malformierter Trailer-Block (`ab5ae5d`) → manuelle Reparatur nötig.
4. 44 Waisen → Drop-or-Rescue-Entscheidung nötig.
5. 15 inhaltsgleiche Branch-Spitzen → Rewrite muss alle 20 Refs gemeinsam betrachten.

Fazit: **kein Branch bereit; alle NICHT-BEREIT.**

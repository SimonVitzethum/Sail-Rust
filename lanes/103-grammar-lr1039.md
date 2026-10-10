# Lane 103 — grammar-lr1039 (ConfigKey/Attrs LR-sauber)
Branch: `lane/103-grammar-lr1039` | Status: AKTIV

## Ziel
Rig-belegte Restkonflikte (Log limited-run-20261010-183338-118019.log lesen!):
sail.lalrpop:1385 (AbstractInst/ConfigKey, `.`) + :1547 (Attrs/RawAttr-Tail).

## Schlüssel-Erkenntnis (nicht raten, nachbauen)
Menhir (`sail/src/lib/parser.mly:1039-1043`): `abstract_instantiation: Eq; Config;
key=separated_nonempty_list(Dot, Id)` — FLACH, Shift-Präferenz löst. Unser explizites
`ConfigKey`-Rekursions-Duplikat erzeugt den Konflikt. Äquivalent LR-sauber bauen
(z.B. Use-Site-lokale Kette ohne leere Produktion im Konfliktzustand, oder Dedup-Nonterminal
je Use-Site gegen LALR-Merge-Ungenauigkeit). Gleiches für Attrs-Tail.

## Aufgaben
- Fix als Code, Tests als Code (NICHT ausführen, E3 — Rig-Loop über Haupt-Session).
  `#![forbid(unsafe_code)]`.

## Done
- Fix als Code (E3, Tests als Code, kein Build): `AbstractInst` mit
  aufgepellerter Erst-Key (`ConfigKeyTail` use-site-lokal) + `RIdDef`
  mit dediziertem `RegAttrs`/`RegAttrsTail` (Use-Site-Dedup).
  Sprache unveraendert (Key-/Attribut-Reihenfolge, `None`-Arm,
  Trailing-`,`, `Some`/`None`-Nutzdaten).
- Beide Stellen konfliktfrei (Rig-Beweis offen → Rig-Loop über
  Haupt-Session; bei Fehlschlag Follow-up mit neuer Rig-Evidenz).

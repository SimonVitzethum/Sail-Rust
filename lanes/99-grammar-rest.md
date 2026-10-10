# Lane 99 — grammar-rest (Konflikt 324-Familie)
Branch: `lane/99-grammar-rest` | Status: AKTIV

## Ziel
Rig-belegten Konflikt `sail.lalrpop:324` (`(<NegNum> ",")+` vs. AtomicTyp-KOptList)
+ Restfamilie (TypList/NumList/PatList, `(...)+`-Makro-Zustände, else-/in-Reste,
AtomSuffix/CallArgs) nach E4-Muster bereinigen. Lane-98-Stand lesen
(`059582c`, Rig-Log `limited-run-20261010-175801-113599.log`, 257 Konflikte).

## Aufgaben
- Fix als Code, Tests als Code (NICHT ausführen, E3 — Rig-Loop über Haupt-Session,
  bei Fehlschlag Follow-up). `#![forbid(unsafe_code)]`.

## Done
- process_root läuft durch (Rig-Beweis offen).
- Lane-99-Stand (E3, Tests als Code, kein Build): alle restlichen
  nullable-Listen strikt (rechts-rekursiv, Trailing-`,`, `?`-Use-Sites wo
  der Port `{}`/`[]`/`()` akzeptierte): TypList (310), NumList (324,
  Headline), PatList, MPatList (1083), FPatList, FMPatList (1143),
  FexpList (910), VecUpdList (935), MapClList, ParamKOptList, EnumFns,
  TypeUnions (1379), SubstList, ExternBindList (neu, aus Inline heraus),
  AttrKVList, AttrDataList (neu, aus Inline heraus). Tuple-Use-Sites
  (Typ/Pat/MPat/Exp) als drei explizite Arme (`(x)`, `(x,)`, `(x, …)`);
  Vektor-/Listen-Inlines wiederverwenden die strikten Listen;
  `(...)+`-Reste (MPatOps-Kette, ConfigKey-`.`, 1282) als explizite
  rechts-rekursive Arme; gemeldete `?`-Schwänze als explizite Arme
  (CallArgs/IdCallTail/PatIdTail/MPatIdTail/MPatSuffix, VecUpd-Tails
  918, FMPat-`=` 1136, UnbracketedTyParam-`constraint` 1220,
  RegDef-`=` 1509). Sprache jeweils unverändert (Ausnahme: degeneriertes
  `instantiation I with` ohne Substitution). Tests: 9 neue
  `parse_*`-Tests + Doppel-Definition `parse_bitfield_body_strict`
  entfernt (Lane-98-Merge-Artefakt, Datei kompilierte nicht).
  GRAMMAR.md Fixes 10–12.
- Offen: Rig-Beweis; bereits-E4-Residuen (377/544/640/648/719/725/
  750/769/889/1363/1395/1404/1423/1429, alle „fertiger Arm vs.
  Fortsetzungs-Shift“) + ungemeldete `?`-Schwänze → Follow-up mit
  Rig-Evidenz.

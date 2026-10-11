# Fremdquellen und Testfixtures

Diese Übersicht erhält Herkunftsinformationen aus den entfernten lokalen
README-Dateien. Maßgeblich bleiben die originalen Lizenz- und Copyrighttexte
in den jeweiligen Quelldateien. Sie werden durch die Dokumentationsbereinigung
nicht verändert. Die Rust-Workspace-Metadaten nennen `MIT OR Apache-2.0`;
übernommene Dateien und Abhängigkeiten besitzen ihre eigenen Bedingungen.

## Eingebettete Sail-Bibliothek

`crates/sail-prelude/sail-lib/` enthält die eingebettete Standardbibliothek
von Sail 0.20.3 einschließlich Unterverzeichnissen. Die originalen
Copyright-/Lizenzheader sind in den Dateien erhalten; beispielsweise steht
in `arith.sail` die BSD-2-Clause-Lizenz. Inventar und Einbettung werden in
`crates/sail-prelude/src/` definiert und durch Korpustests geprüft.
Quelle: [rems-project/sail](https://github.com/rems-project/sail).

## Legacy trace.sail

Datei: `crates/sail-prelude/compat/sail-0.17.1/trace.sail`.
Unveränderte Sail-0.17.1-Version, die ARMv9.4-A benötigt. Sie enthält echte
Fallback-Körper für `enable_tracing`, `disable_tracing` und `is_tracing`.

Quelle: [Sail 0.17.1 trace.sail](https://github.com/rems-project/sail/blob/0.17.1/lib/trace.sail).
SHA-256: `7995b8ff7744e900e72cac1bf1a4a9bf0c065a88d885e96507dcf7ca5b677544`.
Der originale BSD-2-Clause- und Copyrighttext bleibt in der Datei.
Ein explizites `SAIL_RS_LIBDIR` hat Vorrang; fehlende Dateien darin sind Fehler
und führen nicht still zum eingebetteten Fallback.

## ARM ImpDefBool-Regressionsfixture

Datei: `crates/sail-parse/tests/fixtures/arm-impdef-bool.sail`.
Enthält originale Lizenz, Val-Signatur und `ImpDefBool` aus
`sail-arm/arm-v9.4-a/src/impdefs.sail`, Revision
`aacf240a26eeb4b6b39bff81dbfc2e237b8fd683`. Kein Funktionskörper wurde verändert.
Die Datei enthält die originale BSD 3-clause Clear License mit Copyrightangaben.
SHA-256: `e45878fe9338642d146cf02994d9636aa8e6d0117075e3068b2664f3502d41c8`.

Die lange Bedingungskette verursachte einen Stackoverflow im rekursiven
Konverter. Der Test ergänzt die Modell-Fixity `infixl 4 ++`, konvertiert die
vollständige Definition und serialisiert den AST. Er prüft keinen vollständigen
Typcheck des isolierten Fragments, dessen ARM-Deklarationen im Gesamtmodell liegen.

## Lean-Support

`sail-lake` referenziert standardmäßig
[rems-project/lean-sail](https://github.com/rems-project/lean-sail), Revision `v6`,
mit Lean `v4.29.0`. Bei Opt-in für reelle Zahlen wird mathlib `v4.29.0`
referenziert. Pins und Projektlayout stehen im Rust-Quellcode und seinen
Golden-Fixtures; [ARCHITECTURE.md](ARCHITECTURE.md#lean-speichermodell-und-projektlayout)
beschreibt die Einbindung. Der Download oder die Lizenz einer Dependency ist
kein Nachweis für einen erfolgreichen ARM-Lean-Build.

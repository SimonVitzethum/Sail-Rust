# ARENA_REPORT.md — Peak-Vergleich vorher/nachher (Lane 37)

Fixture `phased-temps` (`examples/peak.rs`): 10 Phasen à 20.000 Einträge
à 32 Byte (`Node32`, steht für einen AST-Knoten mit Span + IDs).
Modelliert phasenspezifische Temporaries (z. B. Lowering-Puffer), die nach
Phasenende nicht mehr gebraucht werden.

- **Vorher:** eine einzige `sail-ast`-`GenArena`, Drop erst ganz am Ende
  (kein phasenspezifischer Drop-Punkt).
- **Nachher:** `ShardArena` (`ShardSize::Small`, Budget 1.024/Sharde),
  nach jeder Phase werden alle vollen Shards der beendeten Phase per
  `drop_shard` freigegeben; nur der Anbruch-Shard lebt weiter.

## Messung (2026-10-10, `--release`, je ein Prozess pro Variante)

Reproduktion:

```sh
cargo run -p sail-arena --example peak --release -- before
cargo run -p sail-arena --example peak --release -- after
```

| Größe | vorher | nachher | Delta |
|---|---|---|---|
| Live-Peak (Einträge) | 200.000 | 20.800 | −89,6 % |
| Live-Peak (geschätzt, `live * size_of`) | 6.400.000 B | 665.600 B | −89,6 % |
| Peak-RSS `VmHWM` (echt, Linux) | 11.664 KiB | 3.380 KiB | −8.284 KiB |
| Rest-live am Ende | 200.000 | 320 | — |
| Shards angelegt | 1 (mono) | 196 | — |

Prüfsummen gegen Wegoptimieren: vorher `2000800000` (= exakt
Σ(p+i), alle 200.000 Werte lebten gleichzeitig), nachher `6351520`
(Summe über den 320er-Rest). Beide aktiv verifiziert.

## Einordnung

- Der Live-Peak ist deterministisch und allocator-unabhängig; er ist die
  verbindliche Vergleichsgröße dieser Lane.
- `VmHWM` ist informativ (Prozess-Maximum, inkl. Allocator-Overhead):
  Es belegt, dass die Shard-Drop-Punkte echten RSS-Effekt haben
  (−8,3 MiB bei 6,4 MiB Nutzlast), nicht nur Zähler-Kosmetik.
- Rest-Nachteilprecision: Nachher-Peak (20.800) > Phasengröße (20.000),
  weil der Anbruch-Shard der Vor-Phase bis Phasenende weiterlebt.
  Akzeptiert: Ein Ein-Eintrag-Überhang pro Phasengrenze ist der Preis
  für O(1)-Routing ohne phasenübergreifende Buchhaltung.
- Lane-14-Bezug: Die `ShardSize`-Klassen sind so gewählt, dass
  `aarch64_small`-Shards (`Small`, Budget 1.024) nie überdimensionierte
  Arenen reservieren; `arm-v9.4-a`-Volumina fallen in `Large`
  (Budget 65.536) und damit in drop-bare Shards statt einen Monolithen.

## Done-Kriterium

`cargo test -p sail-arena tuned_peak_is_below_plain_peak` ist grün und
erzwingt `nachher < vorher` als Regressionstest (kleine Variante:
10 × 2.000 Einträge, Peak 20.000 vs. 2.976).

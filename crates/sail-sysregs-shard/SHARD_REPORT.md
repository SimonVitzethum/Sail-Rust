# SHARD_REPORT — Lane 31: `sysregs_autogen.sail` als isolierter Shard

Datei: `sail-arm/arm-v9.4-a/src/sysregs_autogen.sail`
(160302 Zeilen, 8050471 Bytes, 5630 Top-Level-Defs = 2815 `val`/`function`-Paare,
Namen je Paar identisch — per Skript verifiziert, strikte Alternation.)

## Strategie (nur diese Datei)

- Streaming-Split in Top-Level-Definitionen (Klammerzaehlung ohne
  Kommentare/Strings/Chars), Gruppierung in Bloecke zu konstant vielen Defs.
- Pro Block: `val`/`function`-Paarungspruefung + FNV-1a-Fingerprint, danach
  Quelltext verwerfen. **Nie ein Gesamt-Dokument im Speicher.**
- Kein Full-Build, kein Lake — nur parsen + sharden + messen.
- Standard: `defs_per_block = 64` (= 32 Paare, 88 Bloecke).

## Messprotokoll (Erste RAM-Messung fuer Lane 14)

- Datum (UTC): 2026-10-10 ~13:10
- Maschine: 31 GiB RAM, kein Swap; `rustc 1.97.1`, Release-Build.
- Methode: In-Prozess-Peak (`VmHWM` aus `/proc/self/status` am Laufzeitende;
  High-Water-Mark ⇒ Einmallesen == Peak). `/usr/bin/time` ist auf dieser
  Maschine nicht installiert, daher kein externer Gegen-Check moeglich.
- Befehl: `sail-sysregs-shard <datei> --defs-per-block N`
- Wiederholungslauf (N=64): identischer `combined_fingerprint` (`9b7edf2315fe5081`).

| `defs_per_block` | Bloecke | max Block-Bytes | Zeit (ms) | Peak-RSS (`VmHWM`) |
|---|---|---|---|---|
| 16 | 352 | 188493 | 69 | **2872 KiB (2.80 MiB)** |
| 64 (Standard) | 88 | 242115 | 65–78 | **3108 KiB (3.04 MiB)** |
| 256 | 22 | 562219 | 68 | **3520 KiB (3.44 MiB)** |

## Peak-Zahl (Lane-14-Kennzahl)

**Peak-RSS (Standard, 64 Defs/Block): 3108 KiB ≈ 3.0 MiB.**

Der Peak waechst von N=16 bis N=256 nur um ~0.6 MiB (Prozess-Basis, nicht
Daten): Der Speicherverbrauch ist praktisch konstant in der Dateigroesse —
Streaming-Ziel erreicht. Hinweis: `combined_fingerprint` haengt von den
Blockgrenzen ab (Hash ueber Block-Hashes); pro-Block-Fingerprints sind bei
gleicher Konfiguration stabil (Lane-11-Eingabe).

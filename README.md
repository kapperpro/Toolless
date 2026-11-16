# Toolless

Kleine, effiziente Werkzeugsammlung für den Desktop – **vollständig in Rust** geschrieben.
Die Oberfläche ist nativ mit [egui/eframe](https://github.com/emilk/egui) gebaut (kein Webview,
kein Node, kein HTML), orientiert sich an Windows 11 und ist weitgehend anpassbar.

| Modul | Umsetzung |
| --- | --- |
| Spotify-Steuerung | Windows-Mediensteuerung: Cover, Titel, Play/Pause, Vor/Zurück, Spulen, Lautstärke – ohne Login |
| Zwischenablage-Manager | `arboard`: eigene Texte und Bilder, Titel, nach Kategorien gruppiert, Klick = kopieren |
| Notizen | Kategorien mit Unterkategorien, Bilder direkt im Text, Export als .txt / .md / .pdf |
| Rechner | eigener Parser, Tastaturbedienung, löschbarer Verlauf |
| Monitor | Prozesse und Leistung wie im Task-Manager, Verlauf je Komponente, Datenträger, Energie-Übersicht |
| App-Starter | Startmenü-Ordner und -Icons, Favoriten, Sammlungen (ein Klick startet alle), eigene Einträge (.exe, .bat, …) |
| Minecraft | Welten aller gängigen Launcher mit Seed und Version, MCA Selector, Seed-Karten im Browser |
| Einstellungen | Farben (Akzent, Hintergrund, Seitenleiste, Karten), Ecken, Größe, Werkzeuge sortieren/ausblenden |
| Ruhezustand blockieren | `keepawake`, schaltbar in der App und im Tray |
| Autostart | `auto-launch`, startet mit `--minimized` direkt in den Tray |
| Hintergrundbetrieb | Schließen versteckt das Fenster; Tray-Icon (`tray-icon`) mit Menü |

## Starten

Voraussetzung ist nur [Rust](https://rustup.rs) (unter Windows inkl. MSVC Build Tools).

```bash
cargo run            # Entwicklung
cargo build --release
cargo test           # Tests für Rechner, PDF-Export, Notizblöcke, NBT, Minecraft, Leistungsindikatoren
```

## Spotify

Es ist keine Einrichtung nötig. Toolless liest die laufende Wiedergabe über die Mediensteuerung
von Windows (dieselbe Schnittstelle, die auch Wallpaper Engine nutzt) und steuert sie darüber.
Die Lautstärke regelt die Audiositzung von Spotify im Windows-Lautstärkemixer; der Regler
erscheint, sobald Spotify Ton ausgibt. Läuft Spotify nicht, wird die gerade aktive Wiedergabe
(z. B. im Browser) angezeigt.

## Monitor

- **Prozesse:** Apps und Hintergrundprozesse mit Icon, CPU, Arbeitsspeicher und Datenträger;
  Suche, Sortieren per Spaltenkopf, „Task beenden“ mit Rückfrage.
- **Leistung:** Geräteliste mit kleinem Verlauf (CPU, Arbeitsspeicher, jeder Datenträger,
  Netzwerk, jede Grafikkarte), Diagramme und Kennzahlen des gewählten Geräts.
- **Verlauf:** Je Komponente (CPU, Arbeitsspeicher, jede Grafikkarte, jeder Datenträger,
  Netzwerk) über 60 Sekunden bis 7 Tage. Unter dem Mauszeiger
  steht, welches Programm zu dem Zeitpunkt die meiste Last erzeugte. Aufgezeichnet wird auch,
  während Toolless im Tray läuft.
- **Datenträger:** Volumes und die Partitionsaufteilung der physischen Datenträger – nur lesend.
  Ändern geht über die Schaltfläche zur Windows-Datenträgerverwaltung.
- **Energie:** Aktuelle Leistung, Gesamtenergie, CO₂ und Kosten sowie der Verbrauch je Programm
  und Komponente. Gemessen wird, wo es Sensoren gibt: Grafikkarten über den Treiber (NVIDIA: NVML,
  AMD: ADL), der Prozessor über die Energiezähler von Windows, falls das System sie anbietet. Der
  Rest (Arbeitsspeicher, Mainboard, Laufwerke, Netzteilverluste) ist eine Schätzung. Die Seite
  zeigt je Komponente, ob der Wert gemessen oder geschätzt ist. Angelehnt an
  [WattSeal](https://github.com/Daminoup88/WattSeal), aber ohne dessen Code, Kernel-Treiber oder
  Administratorrechte.
- **Grafikkarte:** Die Auslastung kommt aus den Windows-Leistungsindikatoren, die Temperatur vom
  Grafiktreiber (wie im Task-Manager); NVIDIA zusätzlich über NVML.
- **CPU-Temperatur:** Nur, wenn Windows sie herausgibt – auf vielen Systemen erst mit
  Administratorrechten oder gar nicht.

## Minecraft

- **Welten:** Gelesen werden die `saves`-Ordner von Minecraft-Launcher, Modrinth, Prism,
  CurseForge, ATLauncher und GDLauncher sowie eigene Ordner. Seed, Version, Mod-Lader und
  Datapacks stammen aus der `level.dat` der Welt.
- **MCA Selector:** Wird an den üblichen Orten gesucht oder per Pfad festgelegt. MCA Selector
  kennt keinen Startparameter für eine Welt; Toolless drückt deshalb nach dem Start Strg+O und
  tippt den Pfad in den Ordnerdialog (nur Windows).
- **Seed-Karten:** [Chunkbase](https://www.chunkbase.com/apps/seed-map) (Strukturen),
  [mcseedmap.net](https://mcseedmap.net/) (Biome, Seed-Suche) und die
  [Datapack-Karte](https://map.jacobsjo.eu/) (Datapacks und Mods) öffnen im Browser. Eine eigene
  Kartenberechnung enthält Toolless nicht.

## Game Timer

Zählt die Spielzeit je Spiel – automatisch für Spiele aus Steam, Epic, Riot, GOG, Ubisoft, EA und Xbox sowie Minecraft, außerdem für eigene Programme („Eigene App hinzufügen …“ oder „Laufendes Programm hinzufügen“). Rechtsklick auf ein Spiel: ausschließen oder Zeiten löschen. Export als CSV (eine Zeile je Sitzung, in Excel lesbar). Die Erkennung ist eine Heuristik anhand des Programmpfads, es gibt keine Datenbank wie bei Discord.

## Uhr

Wecker (täglich oder einmalig), Countdown, Stoppuhr mit Runden sowie zwei Vollbild-Modi: **AFK** schwärzt alle Bildschirme, **Große Uhr** zeigt zusätzlich die Uhrzeit. Beenden mit F11 oder Esc (optional jede Taste, siehe Einstellungen). Solange ein Modus aktiv ist, verhindert Toolless Standby und Bildschirm-Aus. Wecker klingeln auch, wenn Toolless im Tray läuft.

## Sprache

Einstellungen → Sprache: Deutsch oder English. Texte der Oberfläche werden übersetzt; Fehlermeldungen aus tieferen Programmteilen und das Tray-Menü bleiben teilweise deutsch.

## Daten und Verschlüsselung

Alle Daten liegen im Datenordner des Systems (`%APPDATA%\Toolless` unter Windows):

- `toolless.dat` – gespeicherte Zwischenablage-Einträge und Einstellungen
- `notes/*.json` – eine Datei je Notiz, Bilder unter `notes/images`
- `history.dat` – Minutenverlauf des Monitors (7 Tage)

Die Dateien werden mit dem Windows-Datenschutz (DPAPI) verschlüsselt. Der Schlüssel hängt am
Windows-Konto: Andere Benutzer und andere Rechner können die Dateien nicht lesen – wer unter
deinem Konto angemeldet ist, dagegen schon. Ein eigenes Passwort gibt es nicht. Auf anderen
Systemen als Windows werden die Dateien unverschlüsselt gespeichert.

## Projektstruktur

```
├─ Cargo.toml
├─ assets/                 App-Icon (PNG + Rohdaten für Fenster und Tray)
└─ src/
   ├─ main.rs              Einstieg, Fensteroptionen, Einzelinstanz
   ├─ app.rs               Titelleiste, Seitenleiste, Modulwechsel, Meldungen, 60-fps-Grenze
   ├─ theme.rs             Farbpalette aus den Einstellungen, UI-Bausteine
   ├─ settings.rs          Benutzereinstellungen
   ├─ db.rs, crypto.rs     verschlüsselter Datenspeicher
   ├─ tray.rs              System-Tray
   ├─ platform.rs          Fenster verstecken/anzeigen, runde Ecken, Systemschrift
   ├─ error.rs
   ├─ services/            Fachlogik ohne GUI: spotify, clipboard, notes, pdf, calc,
   │                       launcher, icons, monitor, gpu, pdh, perf_parse, winsys, minecraft, nbt,
   │                       note_blocks, power, autostart
   └─ ui/                  Oberfläche je Modul
```

## Hinweise

- Verlauf: Unter dem Diagramm zeigt eine Zeitleiste einen größeren Bereich (60 s → 1 h, 5 min/1 h → 24 h, 24 h/7 d → 7 d). Per Klick oder Ziehen verschiebst du das Fenster; „Jetzt“ springt zurück.

- **Autostart im Dev-Modus** trägt die Debug-`exe` ein. Für den Alltag den Release-Build verwenden.
- **Andere Systeme:** Mediensteuerung, App-Icons und Verschlüsselung gibt es nur unter Windows;
  unter Linux läuft die App ohne Tray-Icon.

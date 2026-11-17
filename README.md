# Toolless

Kleine, effiziente Werkzeugsammlung für den Desktop – **vollständig in Rust** geschrieben.
Die Oberfläche ist nativ mit [egui/eframe](https://github.com/emilk/egui) gebaut (kein Webview,
kein Node, kein HTML), orientiert sich an Windows 11 und ist weitgehend anpassbar.

Aktuelle Version: **0.9.2** – siehe [CHANGELOG](CHANGELOG.md). Lizenz: [GNU AGPL v3.0](LICENSE).

| Modul | Umsetzung |
| --- | --- |
| Spotify-Steuerung | Windows-Mediensteuerung: Cover, Titel, Play/Pause, Vor/Zurück, Spulen, Lautstärke – ohne Login |
| Zwischenablage-Manager | `arboard`: eigene Texte und Bilder, Titel, nach Kategorien gruppiert, Klick = kopieren |
| Notizen | Kategorien mit Unterkategorien, Bilder direkt im Text, Export als .txt / .md / .pdf |
| Rechner | eigener Parser, Tastaturbedienung, löschbarer Verlauf |
| Task-Manager | Prozesse, Leistung im Stil des Windows-Task-Managers, Autostart, Verlauf je Komponente mit Zeitleiste und Zeitraumfilter, Datenträger, Energie |
| App-Starter | Startmenü-Ordner und -Icons, Favoriten, Sammlungen (ein Klick startet alle), eigene Einträge (.exe, .bat, …) |
| Minecraft | Welten aller gängigen Launcher mit Seed und Version, MCA Selector, Seed-Karten im Browser |
| Uhr | Wecker, mehrere Countdowns und Stoppuhren (Hinzufügen per Fenster), AFK-Bildschirm, große Uhr, Spotify-Vollbild – wählbare Bildschirme, hält den Rechner wach |
| Einstellungen | Zahnrad oben rechts (großes Fenster) oder in der Seitenleiste: Erscheinungsbild (inkl. OLED, hoher Kontrast, eigene Farben) mit Vorschau, Akzentfarbe, Größe, kompakte Darstellung, Werkzeuge ziehen/ausblenden, Sicherung exportieren/importieren |
| Ruhezustand blockieren | `keepawake`, schaltbar in der App und im Tray |
| Autostart | `auto-launch`, startet mit `--minimized` direkt in den Tray |
| Hintergrundbetrieb | Schließen versteckt das Fenster; Tray-Icon (`tray-icon`) mit Menü |

## Installieren

Fertige Installer gibt es unter [Releases](https://github.com/kapperpro/toolless/releases).
Der Installer lässt sich selbst bauen (siehe unten, „Installer erstellen“).

## Starten

Voraussetzung ist nur [Rust](https://rustup.rs) (unter Windows inkl. MSVC Build Tools).

```bash
cargo run            # Entwicklung
cargo build --release
cargo test           # Tests für Rechner, PDF-Export, Notizblöcke, NBT, Minecraft, Leistungsindikatoren
```

## Installer erstellen

1. `cargo build --release` ausführen – es entsteht `target\release\toolless.exe`.
2. `executable\toolless.iss` in [Inno Setup 6](https://jrsoftware.org/isinfo.php) öffnen und kompilieren (Strg+F9).
3. Der Installer liegt danach unter `executable\output\Toolless-Setup-<Version>.exe`.

Die Installation empfiehlt Administratorrechte (nach `Program Files`), kann aber auch nur für den aktuellen Benutzer erfolgen.

## Spotify

Es ist keine Einrichtung nötig. Toolless liest die laufende Wiedergabe über die Mediensteuerung
von Windows (dieselbe Schnittstelle, die auch Wallpaper Engine nutzt) und steuert sie darüber.
Die Lautstärke regelt die Audiositzung von Spotify im Windows-Lautstärkemixer; der Regler
erscheint, sobald Spotify Ton ausgibt. Läuft Spotify nicht, wird die gerade aktive Wiedergabe
(z. B. im Browser) angezeigt.

## Vollbildmodi (Uhr-Tab)

AFK, Große Uhr und Spotify-Vollbild öffnen je gewähltem Bildschirm ein rahmenloses Fenster und
verhindern den Ruhezustand. Beendet wird mit F11, Esc oder einer eigenen Taste (jeweils abschaltbar),
optional mit jeder Taste. Als Notausgang beendet **Esc, 20 Sekunden gehalten**, immer.

## Sicherung

Unter Einstellungen › Daten lässt sich alles in eine Datei exportieren und wieder einspielen
(keine Konten nötig). Die Datei ist nicht verschlüsselt – sicher aufbewahren.

## RTSS / MSI Afterburner

Toolless exportiert das Symbol `RTSSHooksCompatibility` (siehe `build.rs`, nur MSVC). RivaTuner
hängt sich dann nicht ein, Toolless erscheint nicht im Overlay. Der Game Timer ignoriert zudem
Afterburner, RTSS, HWiNFO, OBS und ähnliche Hilfsprogramme.

## Task-Manager

Die Seite „Leistung“ sieht aus wie im Windows-Task-Manager: Gerätelist links, rechts ein 60-Sekunden-Diagramm mit Raster, große Kennzahlen und eine Tabelle mit Hardware-Angaben (Basistakt, Sockets, Kerne, Speichertakt, Steckplätze, Adresse, Treiberversion …). Der Schalter **Erweitert** zeigt zusätzlich die einzelnen logischen Prozessoren, L1/L2/L3-Cache, Virtualisierung, Speicherzusammensetzung, Pools und mehr. Die festen Hardware-Angaben liest ein einmaliger PowerShell-Aufruf (WMI) im Hintergrund; die Zähler (zugesichert, Handles, Threads …) kommen von `GetPerformanceInfo`.

Die Seite „Autostart“ listet die Programme, die mit Windows starten (Registry `Run`, Autostart-Ordner), und schaltet sie wie der Windows-Task-Manager über `StartupApproved` ein oder aus – Einträge für „Alle Benutzer“ brauchen Administratorrechte. In der Prozessliste hebt „Auswahl aufheben“ (oder Esc) die Markierung auf.

Der Messabstand ist oben rechts wählbar (2 s, 1 s, 0,5 s, 0,1 s). Der Verlauf speichert Auslastung und Temperatur in drei Stufen (live, 30 Sekunden, Minuten). Unter „Datenträger“ öffnet ein Klick auf einen Kasten Details wie in CrystalDiskInfo (SMART; NVMe direkt, SATA meist nur mit Administratorrechten).


- **Prozesse:** Apps und Hintergrundprozesse mit Icon, CPU, Arbeitsspeicher und Datenträger;
  Suche, Sortieren per Spaltenkopf, „Task beenden“ mit Rückfrage.
- **Leistung:** Geräteliste mit kleinem Verlauf (CPU, Arbeitsspeicher, jeder Datenträger,
  Netzwerk, jede Grafikkarte), Diagramme und Kennzahlen des gewählten Geräts.
- **Verlauf:** Je Komponente (CPU, Arbeitsspeicher, jede Grafikkarte, jeder Datenträger,
  Netzwerk) über 60 Sekunden bis 30 Tage. Die Aufbewahrungsdauer (1, 3, 7, 14 oder 30 Tage) ist
  wählbar. Unter dem Mauszeiger steht, welches Programm zu dem Zeitpunkt die meiste Last erzeugte.
  Die Zeitleiste darunter hat einen wählbaren Zeitraum, einstellbare Höhe und Zeitmarken mit Datum
  und Uhrzeit; das Fenster verschiebst du per Klick, Ziehen oder mit ◀ ▶. Über „Von“ und „bis“
  lässt sich ein eigener Zeitraum filtern (z. B. `10.12.2025` bis `11.12.2025` oder heute
  `10:00` bis `11:00`), dazu „Heute“ und „Gestern“. Aufgezeichnet wird auch, während Toolless im
  Tray läuft.
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

Zählt die Spielzeit je Spiel – automatisch für Spiele aus Steam, Epic, Riot, GOG, Ubisoft, EA und Xbox sowie Minecraft, außerdem für eigene Programme („Eigene App hinzufügen …“ oder „Laufendes Programm hinzufügen“). Rechtsklick auf ein Spiel: ausschließen oder Zeiten löschen. Export als CSV (eine Zeile je Sitzung, in Excel lesbar). Ein Klick auf einen Balken zeigt die Sitzungen dieses Tages. Aufbewahrungsdauer (24 Stunden bis 365 Tage) und Höchstgröße der Datei lassen sich unten einstellen. Die Erkennung ist eine Heuristik anhand des Programmpfads, es gibt keine Datenbank wie bei Discord.

## Uhr

Wecker (täglich oder einmalig), beliebig viele Countdowns und Stoppuhren mit Runden – neue Wecker und Countdowns entstehen in einem eigenen Fenster. Dazu drei Vollbildmodi: **AFK**, **Große Uhr** und **Spotify**, auf allen oder nur auf gewählten Bildschirmen. Wecker und Countdowns klingeln auch, wenn Toolless im Tray läuft.

## Sprache

Einstellungen → Sprache: Deutsch oder English. Texte der Oberfläche werden übersetzt; Fehlermeldungen aus tieferen Programmteilen und das Tray-Menü bleiben teilweise deutsch.

## Daten und Verschlüsselung

Alle Daten liegen im Datenordner des Systems (`%APPDATA%\Toolless` unter Windows):

- `toolless.dat` – gespeicherte Zwischenablage-Einträge und Einstellungen
- `notes/*.json` – eine Datei je Notiz, Bilder unter `notes/images`
- `history.dat` – Verlauf des Monitors (1 bis 30 Tage, einstellbar; Standard 7 Tage)

Die Dateien werden mit dem Windows-Datenschutz (DPAPI) verschlüsselt. Der Schlüssel hängt am
Windows-Konto: Andere Benutzer und andere Rechner können die Dateien nicht lesen – wer unter
deinem Konto angemeldet ist, dagegen schon. Ein eigenes Passwort gibt es nicht. Auf anderen
Systemen als Windows werden die Dateien unverschlüsselt gespeichert.

## Projektstruktur

```
├─ Cargo.toml, Cargo.lock
├─ build.rs                RTSS-Export (nur MSVC)
├─ LICENSE, CHANGELOG.md, README.md
├─ aufraeumen.ps1          räumt Build-Reste, Sicherungen und gespeicherte Daten auf
├─ assets/                 App-Icon (PNG + Rohdaten für Fenster und Tray)
├─ executable/             Inno-Setup-Skript und Installer-Grafiken
└─ src/
   ├─ main.rs              Einstieg, Fensteroptionen, Einzelinstanz
   ├─ app.rs               Titelleiste, Seitenleiste, Modulwechsel, Meldungen, Aktualisieren (F5)
   ├─ theme.rs             Farbpalette aus den Einstellungen, UI-Bausteine
   ├─ settings.rs          Benutzereinstellungen
   ├─ db.rs, crypto.rs     verschlüsselter Datenspeicher
   ├─ backup.rs            Export/Import aller Daten in eine Datei
   ├─ tray.rs              System-Tray
   ├─ platform.rs          Fenster verstecken/anzeigen, runde Ecken, Systemschrift
   ├─ error.rs
   ├─ services/            Fachlogik ohne GUI: spotify, clipboard, notes, pdf, calc, launcher,
   │                       icons, monitor, hwinfo, startup, gpu, pdh, perf_parse, smart, winsys,
   │                       minecraft, nbt, note_blocks, power, autostart
   └─ ui/                  Oberfläche je Modul (monitor/ enthält Leistung und Autostart)
```

## Hinweise

- **Aktualisieren:** Jeder Tab hat oben rechts einen Button „Aktualisieren“ (Taste F5), der die Ansicht neu lädt.
- **Autostart im Dev-Modus** trägt die Debug-`exe` ein. Für den Alltag den Release-Build verwenden.
- **Andere Systeme:** Mediensteuerung, App-Icons und Verschlüsselung gibt es nur unter Windows;
  unter Linux läuft die App ohne Tray-Icon.

## Lizenz

Toolless steht unter der [GNU Affero General Public License v3.0](LICENSE).

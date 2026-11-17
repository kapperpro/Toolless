# Toolless

A small, efficient collection of desktop tools – written **entirely in Rust**.
The interface is native, built with [egui/eframe](https://github.com/emilk/egui) (no webview,
no Node, no HTML), follows the look of Windows 11 and is highly customisable.

Current version: **0.9.2** – see the [CHANGELOG](CHANGELOG.md). License: [GNU AGPL v3.0](LICENSE).

| Module | Implementation |
| --- | --- |
| Spotify control | Windows media controls: cover, title, play/pause, previous/next, seeking, volume – no login |
| Clipboard manager | `arboard`: your own texts and images, titles, grouped by category, click = copy |
| Notes | Categories with subcategories, images inside the text, export as .txt / .md / .pdf |
| Calculator | Own parser, keyboard control, clearable history |
| Task Manager | Processes, performance in the style of the Windows Task Manager, startup apps, per-component history with timeline and date/time filter, disks, energy |
| App launcher | Start menu folders and icons, favourites, collections (one click starts all), custom entries (.exe, .bat, …) |
| Minecraft | Worlds from all common launchers with seed and version, MCA Selector, seed maps in the browser |
| Clock | Alarms, multiple countdowns and stopwatches (added via a dialog), AFK screen, big clock, Spotify fullscreen – selectable screens, keeps the PC awake |
| Settings | Gear icon top right (large window) or in the sidebar: appearance (incl. OLED, high contrast, custom colours) with preview, accent colour, size, compact layout, drag/hide tools, export/import backup |
| Prevent sleep | `keepawake`, switchable in the app and in the tray |
| Autostart | `auto-launch`, starts with `--minimized` straight into the tray |
| Background mode | Closing hides the window; tray icon (`tray-icon`) with a menu |

## Install

Ready-made installers are available under [Releases](https://github.com/kapperpro/toolless/releases).
You can also build the installer yourself (see "Build the installer" below).

## Run

The only requirement is [Rust](https://rustup.rs) (on Windows including the MSVC Build Tools).

```bash
cargo run            # development
cargo build --release
cargo test           # tests for calculator, PDF export, note blocks, NBT, Minecraft, performance counters
```

## Build the installer

1. Run `cargo build --release` – this creates `target\release\toolless.exe`.
2. Open `executable\toolless.iss` in [Inno Setup 6](https://jrsoftware.org/isinfo.php) and compile it (Ctrl+F9).
3. The installer is then located at `executable\output\Toolless-Setup-<version>.exe`.

The installer recommends administrator rights (installs to `Program Files`) but can also install for the current user only.

## Spotify

No setup is needed. Toolless reads the current playback through the Windows media controls
(the same interface Wallpaper Engine uses) and controls it through them.
The volume slider controls Spotify's audio session in the Windows volume mixer and appears
as soon as Spotify outputs sound. If Spotify is not running, the currently active playback
(e.g. in the browser) is shown.

## Fullscreen modes (Clock tab)

AFK, Big Clock and Spotify fullscreen open a borderless window on every selected screen and
prevent sleep. They are ended with F11, Esc or a custom key (each can be switched off),
optionally with any key. As an emergency exit, **holding Esc for 20 seconds** always ends them.

## Backup

Under Settings › Data you can export everything to one file and import it again
(no accounts needed). The file is not encrypted – keep it safe.

## RTSS / MSI Afterburner

Toolless exports the symbol `RTSSHooksCompatibility` (see `build.rs`, MSVC only). RivaTuner
then does not hook into it, and Toolless does not appear in the overlay. The Game Timer also
ignores Afterburner, RTSS, HWiNFO, OBS and similar helper programs.

## Task Manager

The "Performance" page looks like the Windows Task Manager: device list on the left, a 60-second graph with grid, large figures and a table of hardware details (base clock, sockets, cores, memory speed, slots, address, driver version …) on the right. The **Advanced** switch additionally shows the individual logical processors, L1/L2/L3 cache, virtualisation, memory composition, pools and more. The fixed hardware details are read once by a PowerShell call (WMI) in the background; the counters (committed, handles, threads …) come from `GetPerformanceInfo`.

The "Startup" page lists the programs that start with Windows (registry `Run`, startup folder) and switches them on or off via `StartupApproved`, like the Windows Task Manager – entries for "All users" need administrator rights. In the process list, "Clear selection" (or Esc) removes the highlight.

The measuring interval can be chosen at the top right (2 s, 1 s, 0.5 s, 0.1 s). The history stores load and temperature in three tiers (live, 30 seconds, minutes). Under "Disks", clicking a box opens details like CrystalDiskInfo (SMART; NVMe directly, SATA usually only with administrator rights).

- **Processes:** Apps and background processes with icon, CPU, memory and disk;
  search, sorting by column header, "End task" with confirmation.
- **Performance:** Device list with a small history (CPU, memory, every disk,
  network, every graphics card), graphs and figures of the selected device.
- **History:** Per component (CPU, memory, every graphics card, every disk,
  network) over 60 seconds up to 30 days. The retention time (1, 3, 7, 14 or 30 days) is
  selectable. Under the mouse pointer you see which program caused the most load at that time.
  The timeline below has a selectable span, adjustable height and tick marks with date and time;
  you move the window by clicking, dragging or with ◀ ▶. With "From" and "to" you can filter a
  custom range (e.g. `10.12.2025` to `11.12.2025`, or today `10:00` to `11:00`), plus "Today"
  and "Yesterday". Recording also continues while Toolless runs in the tray.
- **Disks:** Volumes and the partition layout of the physical disks – read-only.
  Changes are made through the button for the Windows Disk Management.
- **Energy:** Current power, total energy, CO₂ and costs as well as consumption per program
  and component. Values are measured where sensors exist: graphics cards via the driver (NVIDIA: NVML,
  AMD: ADL), the processor via the Windows energy counters if the system offers them. The
  rest (memory, mainboard, drives, power supply losses) is an estimate. The page shows for each
  component whether the value is measured or estimated. Inspired by
  [WattSeal](https://github.com/Daminoup88/WattSeal), but without its code, kernel driver or
  administrator rights.
- **Graphics card:** Load comes from the Windows performance counters, temperature from the
  graphics driver (as in the Task Manager); NVIDIA additionally via NVML.
- **CPU temperature:** Only if Windows provides it – on many systems only with
  administrator rights or not at all.

## Minecraft

- **Worlds:** The `saves` folders of the Minecraft Launcher, Modrinth, Prism,
  CurseForge, ATLauncher and GDLauncher as well as custom folders are read. Seed, version, mod loader and
  datapacks come from the world's `level.dat`.
- **MCA Selector:** Searched in the usual places or set by path. MCA Selector
  has no start parameter for a world; after starting it, Toolless therefore presses Ctrl+O and
  types the path into the folder dialog (Windows only).
- **Seed maps:** [Chunkbase](https://www.chunkbase.com/apps/seed-map) (structures),
  [mcseedmap.net](https://mcseedmap.net/) (biomes, seed search) and the
  [Datapack map](https://map.jacobsjo.eu/) (datapacks and mods) open in the browser. Toolless does not
  contain its own map calculation.

## Game Timer

Counts play time per game – automatically for games from Steam, Epic, Riot, GOG, Ubisoft, EA and Xbox as well as Minecraft, and also for your own programs ("Add own app …" or "Add running program"). Right-click a game: exclude it or delete its times. Export as CSV (one row per session, readable in Excel). Clicking a bar shows the sessions of that day. Retention time (24 hours to 365 days) and maximum file size can be set at the bottom. Detection is a heuristic based on the program path; there is no database like Discord has.

## Clock

Alarms (daily or one-time), any number of countdowns and stopwatches with laps – new alarms and countdowns are created in their own window. In addition three fullscreen modes: **AFK**, **Big Clock** and **Spotify**, on all or only selected screens. Alarms and countdowns also ring while Toolless runs in the tray.

## Language

Settings → Language: German or English. Interface texts are translated; error messages from deeper program parts and the tray menu partly stay in German.

## Data and encryption

All data is stored in the system's data folder (`%APPDATA%\Toolless` on Windows):

- `toolless.dat` – saved clipboard entries and settings
- `notes/*.json` – one file per note, images under `notes/images`
- `history.dat` – monitor history (1 to 30 days, adjustable; default 7 days)

The files are encrypted with Windows data protection (DPAPI). The key is tied to the
Windows account: other users and other computers cannot read the files – whoever is signed in
under your account can. There is no password of its own. On systems other than Windows the files
are stored unencrypted.

## Project structure

```
├─ Cargo.toml, Cargo.lock
├─ build.rs                RTSS export (MSVC only)
├─ LICENSE, CHANGELOG.md, README.md
├─ aufraeumen.ps1          cleans up build leftovers, backups and saved data
├─ assets/                 app icon (PNG + raw data for window and tray)
├─ executable/             Inno Setup script and installer graphics
└─ src/
   ├─ main.rs              entry point, window options, single instance
   ├─ app.rs               title bar, sidebar, module switching, messages, refresh (F5)
   ├─ theme.rs             colour palette from the settings, UI building blocks
   ├─ settings.rs          user settings
   ├─ db.rs, crypto.rs     encrypted data store
   ├─ backup.rs            export/import of all data to one file
   ├─ tray.rs              system tray
   ├─ platform.rs          hide/show window, rounded corners, system font
   ├─ error.rs
   ├─ services/            logic without GUI: spotify, clipboard, notes, pdf, calc, launcher,
   │                       icons, monitor, hwinfo, startup, gpu, pdh, perf_parse, smart, winsys,
   │                       minecraft, nbt, note_blocks, power, autostart
   └─ ui/                  interface per module (monitor/ contains Performance and Startup)
```

## Notes

- **Refresh:** Every tab has a "Refresh" button at the top right (key F5) that reloads the view.
- **Autostart in dev mode** registers the debug `exe`. Use the release build for everyday use.
- **Other systems:** Media control, app icons and encryption exist only on Windows;
  on Linux the app runs without a tray icon.

## License

Toolless is licensed under the [GNU Affero General Public License v3.0](LICENSE).

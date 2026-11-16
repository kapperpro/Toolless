//! Minecraft-Helfer: Welten der gängigen Launcher finden, Seed und Version aus
//! `level.dat` lesen, MCA Selector starten und Adressen der Seed-Karten bauen.

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use flate2::read::GzDecoder;

use super::nbt::{self, Nbt};

/// Obergrenze für eine entpackte `level.dat` (üblich sind wenige Kilobyte).
const MAX_LEVEL_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct World {
    pub name: String,
    pub folder: PathBuf,
    /// Launcher, aus dem die Welt stammt (z. B. „Modrinth“).
    pub source: String,
    /// Name des Modpacks bzw. der Instanz; leer beim normalen Minecraft-Launcher.
    pub pack: String,
    /// Anzahl der Mod-Dateien im `mods`-Ordner der Instanz.
    pub mods: usize,
    /// Vorschaubild der Welt (`icon.png`), falls vorhanden.
    pub icon: Option<PathBuf>,
    pub seed: Option<i64>,
    /// Versionsname wie „1.21.4“; leer, wenn die Welt ihn nicht speichert (vor 1.9).
    pub version: String,
    /// Zuletzt gespielt, Millisekunden seit 1970.
    pub last_played_ms: i64,
    /// Mit Mods gespielt (Fabric, Forge, NeoForge, …).
    pub modded: bool,
    /// Mod-Lader laut Welt, z. B. „fabric“.
    pub brands: Vec<String>,
    /// Aktive Datapacks außer dem eingebauten „vanilla“.
    pub datapacks: Vec<String>,
}

fn strings(node: Option<&Nbt>) -> Vec<String> {
    match node {
        Some(Nbt::List(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

/// Baut die Weltbeschreibung aus dem gelesenen `level.dat`.
pub fn world_from_nbt(root: &Nbt, folder: &Path, source: &str) -> World {
    let data = root.get("Data").unwrap_or(root);
    let folder_name = folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    // Seit 1.16 liegt der Seed in den Weltgenerator-Einstellungen, davor direkt in `Data`.
    let seed = data
        .path(&["WorldGenSettings", "seed"])
        .or_else(|| data.get("RandomSeed"))
        .and_then(Nbt::as_i64);
    let brands: Vec<String> = strings(data.get("ServerBrands"))
        .into_iter()
        .filter(|brand| brand != "vanilla")
        .collect();
    let datapacks: Vec<String> = strings(data.path(&["DataPacks", "Enabled"]))
        .into_iter()
        .filter(|pack| pack != "vanilla")
        .collect();
    let was_modded = data.get("WasModded").and_then(Nbt::as_i64).unwrap_or(0) != 0;
    World {
        name: data
            .get("LevelName")
            .and_then(Nbt::as_str)
            .filter(|name| !name.trim().is_empty())
            .map_or(folder_name, str::to_owned),
        folder: folder.to_path_buf(),
        source: source.to_owned(),
        pack: String::new(),
        mods: 0,
        icon: None,
        seed,
        version: data
            .path(&["Version", "Name"])
            .and_then(Nbt::as_str)
            .unwrap_or_default()
            .to_owned(),
        last_played_ms: data.get("LastPlayed").and_then(Nbt::as_i64).unwrap_or(0),
        modded: was_modded || !brands.is_empty(),
        brands,
        datapacks,
    }
}

/// Liest eine Welt aus ihrem Ordner (`level.dat` ist gzip-komprimiertes NBT).
pub fn read_world(folder: &Path, source: &str) -> Option<World> {
    let file = fs::File::open(folder.join("level.dat")).ok()?;
    let mut bytes = Vec::new();
    GzDecoder::new(file)
        .take(MAX_LEVEL_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(world_from_nbt(&nbt::parse(&bytes)?, folder, source))
}

/// Unterordner eines Verzeichnisses (leer, wenn es nicht existiert).
fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect()
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Ein `saves`-Ordner samt Herkunft.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveRoot {
    pub launcher: String,
    /// Modpack bzw. Instanz; leer beim normalen Minecraft-Launcher.
    pub pack: String,
    pub saves: PathBuf,
}

/// Wert einer Zeile `schlüssel=wert` (Prism/MultiMC `instance.cfg`).
pub fn cfg_value(text: &str, key: &str) -> Option<String> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .find(|(name, _)| name.trim() == key)
        .map(|(_, value)| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Text eines JSON-Felds über einen Pfad von Schlüsseln.
fn json_text(file: &Path, keys: &[&str]) -> Option<String> {
    let text = fs::read_to_string(file).ok()?;
    let root = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    let node = keys.iter().try_fold(&root, |node, key| node.get(key))?;
    node.as_str()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
}

/// Name des Modpacks, wie ihn der Launcher im Instanzordner ablegt.
fn pack_name(instance: &Path) -> Option<String> {
    // CurseForge, ATLauncher, ältere Modrinth-App, Prism/MultiMC.
    json_text(&instance.join("minecraftinstance.json"), &["name"])
        .or_else(|| json_text(&instance.join("instance.json"), &["launcher", "name"]))
        .or_else(|| json_text(&instance.join("profile.json"), &["metadata", "name"]))
        .or_else(|| {
            let text = fs::read_to_string(instance.join("instance.cfg")).ok()?;
            cfg_value(&text, "name")
        })
}

/// Anzahl der Mods im Spielordner (aktive `.jar`-Dateien).
fn count_mods(game_dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(game_dir.join("mods")) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("jar"))
        })
        .count()
}

/// `saves`-Ordner aller Instanzen eines Launchers. `inner` sind die möglichen Wege
/// vom Instanzordner zum Spielordner.
fn instance_roots(launcher: &str, instances: &Path, inner: &[&str], out: &mut Vec<SaveRoot>) {
    for instance in subdirs(instances) {
        for sub in inner {
            let saves = instance.join(sub).join("saves");
            if saves.is_dir() {
                out.push(SaveRoot {
                    launcher: launcher.to_owned(),
                    pack: pack_name(&instance).unwrap_or_else(|| name_of(&instance)),
                    saves,
                });
                break;
            }
        }
    }
}

/// Alle `saves`-Ordner: Standard-Launcher, bekannte andere Launcher und die vom
/// Nutzer hinzugefügten Ordner.
pub fn save_roots(extra: &[String]) -> Vec<SaveRoot> {
    let mut roots: Vec<SaveRoot> = Vec::new();
    let mut games: Vec<PathBuf> = Vec::new();
    if let Some(data) = dirs::data_dir() {
        games.push(data.join(".minecraft"));
        instance_roots(
            "Modrinth",
            &data.join("ModrinthApp").join("profiles"),
            &[""],
            &mut roots,
        );
        instance_roots(
            "Modrinth",
            &data.join("com.modrinth.theseus").join("profiles"),
            &[""],
            &mut roots,
        );
        instance_roots(
            "Prism",
            &data.join("PrismLauncher").join("instances"),
            &[".minecraft", "minecraft"],
            &mut roots,
        );
        instance_roots(
            "ATLauncher",
            &data.join("ATLauncher").join("instances"),
            &[""],
            &mut roots,
        );
        instance_roots(
            "GDLauncher",
            &data.join("gdlauncher_next").join("instances"),
            &[""],
            &mut roots,
        );
    }
    if let Some(home) = dirs::home_dir() {
        games.push(home.join(".minecraft"));
        instance_roots(
            "CurseForge",
            &home.join("curseforge").join("minecraft").join("Instances"),
            &[""],
            &mut roots,
        );
    }
    for game in games {
        let saves = game.join("saves");
        if saves.is_dir() {
            roots.insert(
                0,
                SaveRoot {
                    launcher: "Minecraft".to_owned(),
                    pack: String::new(),
                    saves,
                },
            );
        }
    }
    for folder in extra {
        let folder = PathBuf::from(folder);
        let inner = folder.join("saves");
        let saves = if inner.is_dir() { inner } else { folder };
        if !saves.is_dir() {
            continue;
        }
        // Der Instanzordner liegt je nach Launcher ein oder zwei Ebenen über `saves`.
        let game_dir = saves
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| saves.clone());
        let pack = pack_name(&game_dir)
            .or_else(|| game_dir.parent().and_then(pack_name))
            .unwrap_or_else(|| name_of(&game_dir));
        roots.push(SaveRoot {
            launcher: "Eigener Ordner".to_owned(),
            pack,
            saves,
        });
    }
    // Derselbe Ordner kann über mehrere Wege gefunden werden.
    let mut seen: Vec<PathBuf> = Vec::new();
    roots.retain(|root| {
        let unique = !seen.contains(&root.saves);
        seen.push(root.saves.clone());
        unique
    });
    roots
}

/// Liest alle Welten, zuletzt gespielte zuerst.
pub fn scan_worlds(extra: &[String]) -> Vec<World> {
    let mut worlds: Vec<World> = Vec::new();
    for root in save_roots(extra) {
        let mods = root.saves.parent().map_or(0, count_mods);
        // Ein hinzugefügter Ordner darf auch selbst eine einzelne Welt sein.
        let mut folders = vec![root.saves.clone()];
        folders.extend(subdirs(&root.saves));
        for folder in folders {
            if let Some(mut world) = read_world(&folder, &root.launcher) {
                world.pack = root.pack.clone();
                world.mods = mods;
                let icon = folder.join("icon.png");
                world.icon = icon.is_file().then_some(icon);
                worlds.push(world);
            }
        }
    }
    worlds.sort_by(|a, b| b.last_played_ms.cmp(&a.last_played_ms));
    worlds
}

/// Wandelt eine Seed-Eingabe wie Minecraft um: Zahlen direkt, Text über Javas
/// `String.hashCode()`.
pub fn seed_from_text(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(number) = text.parse::<i64>() {
        return Some(number);
    }
    let hash = text.encode_utf16().fold(0i32, |hash, unit| {
        hash.wrapping_mul(31).wrapping_add(unit as i32)
    });
    Some(hash as i64)
}

/// `true` für Versionsnamen der Form `1.21` oder `1.21.4` (keine Snapshots).
pub fn is_release(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    (2..=3).contains(&parts.len())
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

/// Die drei Karten, die sich ergänzen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedMap {
    /// Strukturen-Suche.
    Chunkbase,
    /// Biome und Seed-Suche (cubiomes).
    McSeedMap,
    /// Karte mit Datapack- und Mod-Unterstützung.
    DatapackMap,
}

impl SeedMap {
    pub const ALL: [SeedMap; 3] = [SeedMap::Chunkbase, SeedMap::McSeedMap, SeedMap::DatapackMap];

    pub fn label(self) -> &'static str {
        match self {
            SeedMap::Chunkbase => "Chunkbase",
            SeedMap::McSeedMap => "mcseedmap.net",
            SeedMap::DatapackMap => "Datapack-Karte",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            SeedMap::Chunkbase => {
                "Strukturen (Dörfer, Festungen, Antike Städte …) für jede Version"
            }
            SeedMap::McSeedMap => "Biomkarte und Seed-Suche auf Basis von cubiomes",
            SeedMap::DatapackMap => "Biome und Strukturen auch mit Datapacks und Mods (jacobsjo)",
        }
    }

    /// Adresse der Karte; Seed und Version werden mitgegeben, wo die Seite das kennt.
    pub fn url(self, seed: Option<i64>, version: &str) -> String {
        match (self, seed) {
            (SeedMap::Chunkbase, Some(seed)) => {
                format!("https://www.chunkbase.com/apps/seed-map#seed={seed}")
            }
            (SeedMap::Chunkbase, None) => "https://www.chunkbase.com/apps/seed-map".to_owned(),
            (SeedMap::McSeedMap, Some(seed)) if is_release(version) => {
                format!("https://mcseedmap.net/{version}-Java/{seed}")
            }
            (SeedMap::McSeedMap, _) => "https://mcseedmap.net/".to_owned(),
            (SeedMap::DatapackMap, _) => "https://map.jacobsjo.eu/".to_owned(),
        }
    }
}

pub const MCA_SELECTOR_DOWNLOAD: &str = "https://github.com/Querz/mcaselector/releases/latest";

/// Sucht MCA Selector: zuerst der gespeicherte Pfad, dann die üblichen Installationsorte.
pub fn find_mca_selector(configured: &str) -> Option<PathBuf> {
    let configured = configured.trim();
    if !configured.is_empty() {
        let path = PathBuf::from(configured);
        if path.is_file() {
            return Some(path);
        }
    }
    let mut bases: Vec<PathBuf> = Vec::new();
    if let Some(local) = dirs::data_local_dir() {
        bases.push(local.join("Programs"));
        bases.push(local);
    }
    for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(dir) = std::env::var_os(variable) {
            bases.push(PathBuf::from(dir));
        }
    }
    bases
        .iter()
        .flat_map(|base| {
            ["MCA Selector", "mcaselector"]
                .into_iter()
                .map(move |folder| base.join(folder).join("MCA Selector.exe"))
        })
        .find(|path| path.is_file())
}

/// Startet MCA Selector (`.exe` direkt, `.jar` über Java). Mit `world` wird die Welt
/// danach über den „Welt öffnen“-Dialog geladen (nur Windows, siehe [`automation`]).
/// Liefert `true`, wenn das Öffnen der Welt angestoßen wurde.
pub fn launch_mca_selector(path: &Path, world: Option<&Path>) -> Result<bool, String> {
    let is_jar = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("jar"));
    let mut command = if is_jar {
        let mut command = Command::new(if cfg!(windows) { "javaw" } else { "java" });
        command.arg("-jar").arg(path);
        command
    } else {
        Command::new(path)
    };
    if let Some(dir) = path.parent() {
        command.current_dir(dir);
    }
    // MCA Selector schreibt sein eigenes Protokoll auf die Konsole – das gehört nicht
    // in die Ausgabe von Toolless.
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("MCA Selector ließ sich nicht starten: {err}"))?;

    #[cfg(windows)]
    {
        if let Some(world) = world {
            automation::open_world(world.to_path_buf());
            return Ok(true);
        }
    }
    let _ = world;
    Ok(false)
}

/// MCA Selector kennt keinen Startparameter, der eine Welt in der Oberfläche öffnet.
/// Deshalb wird der Weg des Nutzers nachgestellt: warten, bis das Fenster im
/// Vordergrund ist, Strg+O drücken und den Pfad in den Ordnerdialog tippen.
/// Jeder Schritt prüft vorher, dass wirklich MCA Selector bzw. dessen Dialog den
/// Fokus hat – sonst wird nichts getippt.
#[cfg(windows)]
mod automation {
    use std::{mem, path::PathBuf, thread, time::Duration};

    const INPUT_KEYBOARD: u32 = 1;
    const KEYEVENTF_KEYUP: u32 = 0x2;
    const KEYEVENTF_UNICODE: u32 = 0x4;
    const VK_CONTROL: u16 = 0x11;
    const VK_RETURN: u16 = 0x0D;
    const VK_O: u16 = 0x4F;
    /// Klassenname der Windows-Standarddialoge.
    const DIALOG_CLASS: &str = "#32770";
    const POLL: Duration = Duration::from_millis(250);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct KeyboardInput {
        key: u16,
        scan: u16,
        flags: u32,
        time: u32,
        extra: usize,
    }

    /// `INPUT` mit Tastaturdaten; die Auffüllung bringt es auf die Größe der
    /// Windows-Struktur (deren größtes Mitglied die Mausdaten sind).
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Input {
        kind: u32,
        keyboard: KeyboardInput,
        padding: [u8; 8],
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetForegroundWindow() -> isize;
        fn GetWindowTextW(window: isize, text: *mut u16, max: i32) -> i32;
        fn GetClassNameW(window: isize, text: *mut u16, max: i32) -> i32;
        fn SendInput(count: u32, inputs: *const Input, size: i32) -> u32;
    }

    fn key(key: u16, scan: u16, flags: u32) -> Input {
        Input {
            kind: INPUT_KEYBOARD,
            keyboard: KeyboardInput {
                key,
                scan,
                flags,
                time: 0,
                extra: 0,
            },
            padding: [0; 8],
        }
    }

    fn send(inputs: &[Input]) {
        // SAFETY: `inputs` ist ein gültiger Bereich von `Input`-Strukturen, deren Aufbau
        // und Größe der Windows-Definition entsprechen.
        unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                mem::size_of::<Input>() as i32,
            );
        }
    }

    /// Titel und Klassenname des Fensters im Vordergrund.
    fn foreground() -> (String, String) {
        let mut title = [0u16; 256];
        let mut class = [0u16; 128];
        // SAFETY: Beide Puffer sind so groß wie angegeben; ein ungültiges Fenster
        // liefert die Länge 0.
        let (title_len, class_len) = unsafe {
            let window = GetForegroundWindow();
            (
                GetWindowTextW(window, title.as_mut_ptr(), title.len() as i32),
                GetClassNameW(window, class.as_mut_ptr(), class.len() as i32),
            )
        };
        (
            String::from_utf16_lossy(&title[..title_len.max(0) as usize]),
            String::from_utf16_lossy(&class[..class_len.max(0) as usize]),
        )
    }

    /// Wartet, bis das Fenster im Vordergrund die Bedingung erfüllt.
    fn wait_for(seconds: u64, matches: impl Fn(&str, &str) -> bool) -> bool {
        for _ in 0..seconds * 4 {
            let (title, class) = foreground();
            if matches(&title, &class) {
                return true;
            }
            thread::sleep(POLL);
        }
        false
    }

    pub fn open_world(world: PathBuf) {
        thread::spawn(move || {
            // 1. Das Hauptfenster muss erscheinen und den Fokus haben.
            if !wait_for(45, |title, class| {
                title.contains("MCA Selector") && class != DIALOG_CLASS
            }) {
                return;
            }
            thread::sleep(Duration::from_millis(1500));
            if !foreground().0.contains("MCA Selector") {
                return;
            }

            // 2. Strg+O öffnet „Welt öffnen“.
            send(&[
                key(VK_CONTROL, 0, 0),
                key(VK_O, 0, 0),
                key(VK_O, 0, KEYEVENTF_KEYUP),
                key(VK_CONTROL, 0, KEYEVENTF_KEYUP),
            ]);
            if !wait_for(8, |_, class| class == DIALOG_CLASS) {
                return;
            }
            thread::sleep(Duration::from_millis(700));
            if foreground().1 != DIALOG_CLASS {
                return;
            }

            // 3. Pfad in das Feld „Ordner“ tippen und bestätigen.
            let mut inputs: Vec<Input> = Vec::new();
            for unit in world.to_string_lossy().encode_utf16() {
                inputs.push(key(0, unit, KEYEVENTF_UNICODE));
                inputs.push(key(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
            }
            send(&inputs);
            thread::sleep(Duration::from_millis(300));
            send(&[key(VK_RETURN, 0, 0), key(VK_RETURN, 0, KEYEVENTF_KEYUP)]);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Nbt {
        Nbt::String(value.to_owned())
    }

    fn compound(children: Vec<(&str, Nbt)>) -> Nbt {
        Nbt::Compound(
            children
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        )
    }

    #[test]
    fn reads_modern_world() {
        let root = compound(vec![(
            "Data",
            compound(vec![
                ("LevelName", text("Survival")),
                ("LastPlayed", Nbt::Long(1_700_000_000_000)),
                ("WorldGenSettings", compound(vec![("seed", Nbt::Long(-77))])),
                ("Version", compound(vec![("Name", text("1.21.4"))])),
                (
                    "ServerBrands",
                    Nbt::List(vec![text("vanilla"), text("fabric")]),
                ),
                (
                    "DataPacks",
                    compound(vec![(
                        "Enabled",
                        Nbt::List(vec![text("vanilla"), text("file/terralith.zip")]),
                    )]),
                ),
            ]),
        )]);
        let world = world_from_nbt(&root, Path::new("saves/ordner"), "Minecraft");
        assert_eq!(world.name, "Survival");
        assert_eq!(world.seed, Some(-77));
        assert_eq!(world.version, "1.21.4");
        assert!(world.modded);
        assert_eq!(world.brands, ["fabric"]);
        assert_eq!(world.datapacks, ["file/terralith.zip"]);
    }

    #[test]
    fn reads_old_world_and_falls_back_to_folder_name() {
        let root = compound(vec![("Data", compound(vec![("RandomSeed", Nbt::Long(5))]))]);
        let world = world_from_nbt(&root, Path::new("saves/Alte Welt"), "Minecraft");
        assert_eq!(world.name, "Alte Welt");
        assert_eq!(world.seed, Some(5));
        assert!(world.version.is_empty());
        assert!(!world.modded);
    }

    #[test]
    fn reads_instance_cfg() {
        let text = "InstanceType=OneSix\nname=All the Mods 9\niconKey=atm9\n";
        assert_eq!(cfg_value(text, "name").as_deref(), Some("All the Mods 9"));
        assert_eq!(cfg_value(text, "iconKey").as_deref(), Some("atm9"));
        assert_eq!(cfg_value(text, "fehlt"), None);
    }

    #[test]
    fn seeds_like_minecraft() {
        assert_eq!(seed_from_text(" 123 "), Some(123));
        assert_eq!(seed_from_text("-9"), Some(-9));
        // Javas "abc".hashCode() == 96354, "Glacier".hashCode() ist negativ möglich.
        assert_eq!(seed_from_text("abc"), Some(96_354));
        assert_eq!(seed_from_text(""), None);
    }

    #[test]
    fn builds_map_urls() {
        assert!(
            is_release("1.21.4") && is_release("1.20") && !is_release("24w14a") && !is_release("")
        );
        assert_eq!(
            SeedMap::Chunkbase.url(Some(-5), "1.21"),
            "https://www.chunkbase.com/apps/seed-map#seed=-5"
        );
        assert_eq!(
            SeedMap::McSeedMap.url(Some(7), "1.21.4"),
            "https://mcseedmap.net/1.21.4-Java/7"
        );
        assert_eq!(
            SeedMap::McSeedMap.url(Some(7), "24w14a"),
            "https://mcseedmap.net/"
        );
    }
}

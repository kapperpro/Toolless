//! Spotify-Steuerung über die Mediensteuerung des Betriebssystems.
//!
//! Windows stellt jede laufende Wiedergabe (Spotify, Browser, …) über die
//! „System Media Transport Controls“ bereit – dieselbe Schnittstelle, die auch
//! Wallpaper Engine und die Lautstärke-Einblendung nutzen. Es ist weder eine
//! Anmeldung noch eine eigene Spotify-App nötig: Läuft Spotify, werden Titel
//! Interpret und Cover gelesen und Play/Pause, Vor und Zurück direkt gesteuert.
//! Die Lautstärke wird über die Audiositzung von Spotify im Windows-Mixer geregelt.
//!
//! Die Abfragen laufen in einem eigenen Worker-Thread; die GUI schickt
//! Kommandos und liest einen Schnappschuss des Zustands.

use std::{
    process,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
        Arc, Mutex, MutexGuard,
    },
    thread,
    time::{Duration, Instant},
};

use eframe::egui;

use crate::error::{AppError, AppResult};

const POLL_INTERVAL: Duration = Duration::from_millis(1000);

/// Aufträge der GUI (bzw. des Tray-Menüs) an den Worker.
#[derive(Debug, Clone, Copy)]
pub enum Command {
    Toggle,
    Next,
    Previous,
    /// Lautstärke der Spotify-Sitzung, 0.0 – 1.0.
    Volume(f32),
    /// Zur Position (Millisekunden) im laufenden Titel springen.
    Seek(i64),
}

/// Album-Cover als unkomprimierte RGBA-Pixel.
#[derive(Debug)]
pub struct Artwork {
    /// Ändert sich mit jedem neuen Cover – die GUI lädt die Textur nur dann neu.
    pub id: u64,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlaybackInfo {
    pub is_playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub progress_ms: i64,
    pub duration_ms: i64,
    /// Anzeigename der Quelle, z. B. „Spotify“.
    pub source: String,
    pub is_spotify: bool,
}

/// Zustand, den die GUI anzeigt.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub playback: Option<PlaybackInfo>,
    /// Zeitpunkt, zu dem `playback` gelesen wurde (für eine flüssige Fortschrittsanzeige).
    pub captured_at: Option<Instant>,
    pub artwork: Option<Arc<Artwork>>,
    /// Lautstärke der Spotify-Sitzung (0.0 – 1.0), falls Spotify gerade Ton ausgibt.
    pub volume: Option<f32>,
    /// Letzter Fehler; wird von der GUI einmalig abgeholt.
    pub error: Option<String>,
}

/// Griff der GUI auf den Medien-Worker.
pub struct SpotifyHandle {
    tx: Sender<Command>,
    shared: Arc<Mutex<Snapshot>>,
    polling: Arc<AtomicBool>,
}

fn lock(shared: &Mutex<Snapshot>) -> MutexGuard<'_, Snapshot> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl SpotifyHandle {
    pub fn spawn(repaint: egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Snapshot::default()));
        let polling = Arc::new(AtomicBool::new(false));

        let worker = Worker {
            rx,
            shared: Arc::clone(&shared),
            polling: Arc::clone(&polling),
            repaint,
            art_key: String::new(),
            art_loaded: false,
            art_attempts: 0,
            art_counter: 0,
        };
        if let Err(err) = thread::Builder::new()
            .name("media".into())
            .spawn(move || worker.run())
        {
            lock(&shared).error = Some(format!("Medien-Worker konnte nicht starten: {err}"));
        }

        Self {
            tx,
            shared,
            polling,
        }
    }

    pub fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }

    /// Sender für andere Auslöser (Tray-Menü).
    pub fn sender(&self) -> Sender<Command> {
        self.tx.clone()
    }

    pub fn snapshot(&self) -> Snapshot {
        lock(&self.shared).clone()
    }

    pub fn take_error(&self) -> Option<String> {
        lock(&self.shared).error.take()
    }

    /// Regelmäßiges Abfragen nur, solange die Spotify-Ansicht sichtbar ist.
    pub fn set_polling(&self, enabled: bool) {
        self.polling.store(enabled, Ordering::Relaxed);
    }
}

struct Worker {
    rx: Receiver<Command>,
    shared: Arc<Mutex<Snapshot>>,
    polling: Arc<AtomicBool>,
    repaint: egui::Context,
    /// Titel, für den das Cover zuletzt angefragt wurde.
    art_key: String,
    art_loaded: bool,
    art_attempts: u8,
    art_counter: u64,
}

/// Das Cover steht oft erst kurz nach dem Titelwechsel bereit → ein paar Versuche.
const ART_ATTEMPTS: u8 = 6;

impl Worker {
    fn run(mut self) {
        backend::init_thread();
        self.publish(Ok(()));
        loop {
            let result = match self.rx.recv_timeout(POLL_INTERVAL) {
                Ok(Command::Volume(level)) => backend::set_volume(level.clamp(0.0, 1.0)),
                Ok(command) => {
                    let result = backend::control(command);
                    // Der Player braucht einen Moment, bis der neue Zustand abrufbar ist.
                    thread::sleep(Duration::from_millis(250));
                    result
                }
                Err(RecvTimeoutError::Timeout) => {
                    if !self.polling.load(Ordering::Relaxed) {
                        continue;
                    }
                    Ok(())
                }
                Err(RecvTimeoutError::Disconnected) => return,
            };
            self.publish(result);
        }
    }

    /// Lädt das Cover neu, wenn ein anderer Titel läuft. `Some(…)` = Cover hat sich geändert.
    fn refresh_artwork(&mut self, playback: Option<&PlaybackInfo>) -> Option<Option<Arc<Artwork>>> {
        let key = playback
            .map(|pb| format!("{}\n{}\n{}", pb.title, pb.artist, pb.album))
            .unwrap_or_default();
        let mut change = None;
        if key != self.art_key {
            self.art_key = key;
            self.art_loaded = false;
            self.art_attempts = 0;
            change = Some(None); // altes Cover sofort entfernen
        }
        if !self.art_loaded && !self.art_key.is_empty() && self.art_attempts < ART_ATTEMPTS {
            self.art_attempts += 1;
            if let Some((width, height, rgba)) =
                backend::artwork().and_then(|bytes| decode_artwork(&bytes))
            {
                self.art_counter += 1;
                self.art_loaded = true;
                change = Some(Some(Arc::new(Artwork {
                    id: self.art_counter,
                    width,
                    height,
                    rgba,
                })));
            }
        }
        change
    }

    /// Überträgt den aktuellen Zustand an die GUI und fordert ein Neuzeichnen an.
    fn publish(&mut self, result: AppResult<()>) {
        let playback = backend::playback();
        let captured_at = Instant::now();
        let artwork = self.refresh_artwork(playback.as_ref().ok().and_then(|pb| pb.as_ref()));
        let volume = backend::volume();
        {
            let mut state = lock(&self.shared);
            state.captured_at = Some(captured_at);
            state.volume = volume;
            if let Some(artwork) = artwork {
                state.artwork = artwork;
            }
            match playback {
                Ok(playback) => state.playback = playback,
                Err(err) => {
                    state.playback = None;
                    // Lesefehler nur melden, wenn nicht schon die Aktion selbst fehlschlug.
                    if result.is_ok() && state.error.is_none() {
                        state.error = Some(err.to_string());
                    }
                }
            }
            if let Err(err) = result {
                state.error = Some(err.to_string());
            }
        }
        self.repaint.request_repaint();
    }
}

/// Dekodiert das Cover (JPEG/PNG) und verkleinert es auf eine handliche Größe.
fn decode_artwork(bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let image = image::load_from_memory(bytes)
        .ok()?
        .thumbnail(360, 360)
        .to_rgba8();
    let (width, height) = (image.width() as usize, image.height() as usize);
    Some((width, height, image.into_raw()))
}

/// Öffnet die Spotify-App über ihr URI-Schema.
pub fn open_spotify() -> AppResult<()> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = process::Command::new("explorer");
        command.arg("spotify:");
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = process::Command::new("open");
        command.arg("spotify:");
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = process::Command::new("xdg-open");
        command.arg("spotify:");
        command
    };

    command
        .spawn()
        .map(|_| ())
        .map_err(|e| AppError::Media(format!("Spotify konnte nicht geöffnet werden: {e}")))
}

#[cfg(windows)]
mod backend {
    use std::time::{SystemTime, UNIX_EPOCH};

    use windows::{
        core::Interface,
        Media::Control::{
            GlobalSystemMediaTransportControlsSession as Session,
            GlobalSystemMediaTransportControlsSessionManager as Manager,
            GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
        },
        Storage::Streams::{DataReader, IInputStream},
        Win32::{
            Media::Audio::{
                eMultimedia, eRender, IAudioSessionControl2, IAudioSessionManager2,
                IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator,
            },
            System::Com::{
                CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED,
            },
        },
    };

    use super::{Command, PlaybackInfo};
    use crate::error::{AppError, AppResult};

    /// 100-Nanosekunden-Schritte pro Millisekunde (Windows-Zeitformat).
    const TICKS_PER_MS: i64 = 10_000;
    /// Abstand zwischen 1601 (Windows-Epoche) und 1970 (Unix-Epoche) in Ticks.
    const EPOCH_OFFSET_TICKS: i64 = 116_444_736_000_000_000;

    fn win<T>(result: windows::core::Result<T>) -> AppResult<T> {
        result.map_err(|err| AppError::Media(err.to_string()))
    }

    /// Größte akzeptierte Cover-Datei (Schutz vor unsinnigen Größenangaben).
    const MAX_ARTWORK_BYTES: u64 = 8 * 1024 * 1024;

    /// COM für den Worker-Thread initialisieren (Voraussetzung für die Audio-Schnittstellen).
    pub fn init_thread() {
        // SAFETY: Standard-Initialisierung; der Thread lebt bis zum Programmende.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
    }

    /// Rohdaten (JPEG/PNG) des Covers der aktuellen Wiedergabe.
    pub fn artwork() -> Option<Vec<u8>> {
        let (session, _, _) = find_session().ok()??;
        let properties = session.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
        let stream = properties
            .Thumbnail()
            .ok()?
            .OpenReadAsync()
            .ok()?
            .get()
            .ok()?;
        let size = stream.Size().ok()?;
        if size == 0 || size > MAX_ARTWORK_BYTES {
            return None;
        }
        let input: IInputStream = stream.cast().ok()?;
        let reader = DataReader::CreateDataReader(&input).ok()?;
        reader.LoadAsync(size as u32).ok()?.get().ok()?;
        let mut bytes = vec![0u8; size as usize];
        reader.ReadBytes(&mut bytes).ok()?;
        Some(bytes)
    }

    /// Lautstärkeregler aller Audiositzungen, die zu Spotify gehören.
    fn spotify_volumes() -> windows::core::Result<Vec<ISimpleAudioVolume>> {
        let mut volumes = Vec::new();
        // SAFETY: reine COM-Aufrufe auf gültigen, von Windows gelieferten Schnittstellen.
        // Die von `GetSessionIdentifier` gelieferte Zeichenkette wird nach dem Kopieren
        // mit `CoTaskMemFree` freigegeben, wie es die API verlangt.
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
            let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
            let sessions = manager.GetSessionEnumerator()?;
            for index in 0..sessions.GetCount()? {
                let Ok(control) = sessions.GetSession(index) else {
                    continue;
                };
                let Ok(details) = control.cast::<IAudioSessionControl2>() else {
                    continue;
                };
                let Ok(identifier) = details.GetSessionIdentifier() else {
                    continue;
                };
                // Die Kennung enthält den Pfad der ausgebenden Anwendung.
                let name = identifier.to_string().unwrap_or_default().to_lowercase();
                CoTaskMemFree(Some(identifier.0 as *const _));
                if name.contains("spotify") {
                    if let Ok(volume) = control.cast::<ISimpleAudioVolume>() {
                        volumes.push(volume);
                    }
                }
            }
        }
        Ok(volumes)
    }

    /// Aktuelle Lautstärke von Spotify (0.0 – 1.0); `None`, wenn Spotify keinen Ton ausgibt.
    pub fn volume() -> Option<f32> {
        let volumes = spotify_volumes().ok()?;
        // SAFETY: gültige Schnittstelle aus `spotify_volumes`.
        volumes
            .first()
            .and_then(|volume| unsafe { volume.GetMasterVolume().ok() })
    }

    pub fn set_volume(level: f32) -> AppResult<()> {
        let volumes = win(spotify_volumes())?;
        if volumes.is_empty() {
            return Err(AppError::Media(
                "Spotify gibt gerade keinen Ton aus – starte dort die Wiedergabe.".into(),
            ));
        }
        for volume in volumes {
            // SAFETY: gültige Schnittstelle; ein Null-Kontext ist laut API erlaubt.
            win(unsafe { volume.SetMasterVolume(level, std::ptr::null()) })?;
        }
        Ok(())
    }

    fn now_ticks() -> i64 {
        let since_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() / 100)
            .unwrap_or(0);
        since_unix as i64 + EPOCH_OFFSET_TICKS
    }

    /// Bevorzugt die Spotify-Sitzung; sonst die aktuell aktive Wiedergabe.
    fn find_session() -> AppResult<Option<(Session, String, bool)>> {
        let manager = win(win(Manager::RequestAsync())?.get())?;

        let sessions = win(manager.GetSessions())?;
        let count = win(sessions.Size())?;
        for index in 0..count {
            let Ok(session) = sessions.GetAt(index) else {
                continue;
            };
            let app_id = session
                .SourceAppUserModelId()
                .map(|id| id.to_string())
                .unwrap_or_default();
            if app_id.to_lowercase().contains("spotify") {
                return Ok(Some((session, "Spotify".to_owned(), true)));
            }
        }

        // Kein Spotify: aktuelle Sitzung (kann fehlen → kein Fehler).
        let Ok(session) = manager.GetCurrentSession() else {
            return Ok(None);
        };
        let app_id = session
            .SourceAppUserModelId()
            .map(|id| id.to_string())
            .unwrap_or_default();
        // „chrome.exe“ → „chrome“, „Firma.App_abc!App“ → „Firma.App“
        let name = app_id
            .split(['!', '_'])
            .next()
            .unwrap_or("")
            .trim_end_matches(".exe")
            .to_owned();
        Ok(Some((session, name, false)))
    }

    pub fn playback() -> AppResult<Option<PlaybackInfo>> {
        let Some((session, source, is_spotify)) = find_session()? else {
            return Ok(None);
        };

        let properties = win(win(session.TryGetMediaPropertiesAsync())?.get())?;
        let text = |value: windows::core::Result<windows::core::HSTRING>| {
            value.map(|s| s.to_string()).unwrap_or_default()
        };

        let is_playing = session
            .GetPlaybackInfo()
            .and_then(|info| info.PlaybackStatus())
            .map(|status| status == Status::Playing)
            .unwrap_or(false);

        let (mut progress_ms, mut duration_ms) = (0, 0);
        if let Ok(timeline) = session.GetTimelineProperties() {
            let start = timeline.StartTime().map(|t| t.Duration).unwrap_or(0);
            let end = timeline.EndTime().map(|t| t.Duration).unwrap_or(0);
            let position = timeline.Position().map(|t| t.Duration).unwrap_or(0);
            let updated = timeline
                .LastUpdatedTime()
                .map(|t| t.UniversalTime)
                .unwrap_or(0);

            duration_ms = (end - start).max(0) / TICKS_PER_MS;
            let mut position = position - start;
            // Die Position wird nur bei Zustandswechseln gemeldet – dazwischen hochrechnen.
            if is_playing && updated > 0 {
                position += (now_ticks() - updated).max(0);
            }
            progress_ms = (position.max(0) / TICKS_PER_MS).min(duration_ms.max(0));
        }

        Ok(Some(PlaybackInfo {
            is_playing,
            title: text(properties.Title()),
            artist: text(properties.Artist()),
            album: text(properties.AlbumTitle()),
            progress_ms,
            duration_ms,
            source,
            is_spotify,
        }))
    }

    pub fn control(command: Command) -> AppResult<()> {
        let Some((session, _, _)) = find_session()? else {
            return Err(AppError::Media(
                "Keine Wiedergabe gefunden. Öffne Spotify und starte einen Titel.".into(),
            ));
        };
        let accepted = match command {
            Command::Toggle => win(win(session.TryTogglePlayPauseAsync())?.get())?,
            Command::Next => win(win(session.TrySkipNextAsync())?.get())?,
            Command::Previous => win(win(session.TrySkipPreviousAsync())?.get())?,
            // Windows rechnet in 100-Nanosekunden-Schritten.
            Command::Seek(position_ms) => {
                let ticks = position_ms.max(0).saturating_mul(10_000);
                let moved = win(win(session.TryChangePlaybackPositionAsync(ticks))?.get())?;
                if !moved {
                    return Err(AppError::Media(
                        "Dieser Player erlaubt das Spulen über Windows nicht.".into(),
                    ));
                }
                true
            }
            Command::Volume(_) => true,
        };
        if accepted {
            Ok(())
        } else {
            Err(AppError::Media(
                "Der Player hat den Befehl nicht angenommen.".into(),
            ))
        }
    }
}

#[cfg(not(windows))]
mod backend {
    use super::{Command, PlaybackInfo};
    use crate::error::{AppError, AppResult};

    const UNSUPPORTED: &str = "Die Mediensteuerung wird derzeit nur unter Windows unterstützt.";

    pub fn init_thread() {}

    pub fn playback() -> AppResult<Option<PlaybackInfo>> {
        Ok(None)
    }

    pub fn artwork() -> Option<Vec<u8>> {
        None
    }

    pub fn volume() -> Option<f32> {
        None
    }

    pub fn set_volume(_level: f32) -> AppResult<()> {
        Err(AppError::Media(UNSUPPORTED.into()))
    }

    pub fn control(_command: Command) -> AppResult<()> {
        Err(AppError::Media(UNSUPPORTED.into()))
    }
}

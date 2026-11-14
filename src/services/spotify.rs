//! Spotify-Steuerung über die Spotify Web API (rspotify, Authorization Code + PKCE).
//!
//! PKCE benötigt kein Client-Secret – es genügt die Client-ID einer eigenen App
//! aus dem Spotify Developer Dashboard. Alle Netzwerkaufrufe laufen in einem
//! eigenen Worker-Thread, damit die Oberfläche nie blockiert. Die GUI schickt
//! Kommandos und liest einen Schnappschuss des Zustands.

use std::{
    io::{BufRead, BufReader, ErrorKind, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
        Arc, Mutex, MutexGuard,
    },
    thread,
    time::{Duration, Instant},
};

use eframe::egui;
use rspotify::{
    model::{AdditionalType, PlayableItem},
    prelude::*,
    scopes, AuthCodePkceSpotify, Config, Credentials, OAuth,
};
use url::Url;

use crate::{
    db::Db,
    error::{AppError, AppResult},
};

const REDIRECT_PORT: u16 = 8888;
/// Muss exakt so im Spotify Developer Dashboard als Redirect URI eingetragen sein.
pub const REDIRECT_URI: &str = "http://127.0.0.1:8888/callback";
const SETTING_CLIENT_ID: &str = "spotify_client_id";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(180);
const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// Aufträge der GUI (bzw. des Tray-Menüs) an den Worker.
#[derive(Debug, Clone)]
pub enum Command {
    Login(String),
    Logout,
    Toggle,
    Next,
    Previous,
    Volume(u8),
    Refresh,
}

#[derive(Debug, Clone, Default)]
pub struct PlaybackInfo {
    pub is_playing: bool,
    pub title: String,
    pub artist: String,
    pub progress_ms: i64,
    pub duration_ms: i64,
    pub device: String,
    pub volume: Option<u32>,
}

/// Zustand, den die GUI anzeigt.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub authenticated: bool,
    pub connecting: bool,
    pub client_id: String,
    pub playback: Option<PlaybackInfo>,
    /// Letzter Fehler; wird von der GUI einmalig abgeholt.
    pub error: Option<String>,
}

/// Griff der GUI auf den Spotify-Worker.
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
    pub fn spawn(db: Arc<Db>, cache_path: PathBuf, repaint: egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Snapshot::default()));
        let polling = Arc::new(AtomicBool::new(false));

        let worker = Worker {
            rx,
            shared: Arc::clone(&shared),
            polling: Arc::clone(&polling),
            db,
            cache_path,
            repaint,
            client: None,
        };
        if let Err(err) = thread::Builder::new()
            .name("spotify".into())
            .spawn(move || worker.run())
        {
            lock(&shared).error = Some(format!("Spotify-Worker konnte nicht starten: {err}"));
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
    db: Arc<Db>,
    cache_path: PathBuf,
    repaint: egui::Context,
    client: Option<AuthCodePkceSpotify>,
}

impl Worker {
    fn run(mut self) {
        self.restore();
        self.publish(Ok(()));

        loop {
            let command = match self.rx.recv_timeout(POLL_INTERVAL) {
                Ok(command) => Some(command),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            };
            let result = match command {
                None => {
                    if !self.polling.load(Ordering::Relaxed) || self.client.is_none() {
                        continue;
                    }
                    Ok(())
                }
                Some(Command::Refresh) => Ok(()),
                Some(Command::Login(client_id)) => self.login(&client_id),
                Some(Command::Logout) => self.logout(),
                Some(control) => self.control(control),
            };
            self.publish(result);
        }
    }

    /// Überträgt den aktuellen Zustand an die GUI und fordert ein Neuzeichnen an.
    fn publish(&self, result: AppResult<()>) {
        let playback = self
            .client
            .as_ref()
            .and_then(|client| fetch_playback(client).ok().flatten());
        {
            let mut state = lock(&self.shared);
            state.authenticated = self.client.is_some();
            state.connecting = false;
            state.playback = playback;
            if let Err(err) = result {
                state.error = Some(err.to_string());
            }
        }
        self.repaint.request_repaint();
    }

    /// Stellt eine frühere Anmeldung aus dem Token-Cache wieder her.
    fn restore(&mut self) {
        let Ok(Some(client_id)) = self.db.get_setting(SETTING_CLIENT_ID) else {
            return;
        };
        lock(&self.shared).client_id = client_id.clone();

        let client = build_client(&client_id, self.cache_path.clone());
        // Abgelaufene Tokens sind in Ordnung – sie werden beim ersten Aufruf erneuert.
        if let Ok(Some(token)) = client.read_token_cache(true) {
            if let Ok(mut slot) = client.token.lock() {
                *slot = Some(token);
            }
            self.client = Some(client);
        }
    }

    /// PKCE-Login: öffnet den Browser und wartet lokal auf die Weiterleitung.
    fn login(&mut self, client_id: &str) -> AppResult<()> {
        let client_id = client_id.trim();
        if client_id.is_empty() {
            return Err(AppError::Spotify("Bitte eine Client-ID eintragen.".into()));
        }
        lock(&self.shared).connecting = true;
        self.repaint.request_repaint();

        let mut client = build_client(client_id, self.cache_path.clone());
        let auth_url = client.get_authorize_url(None)?;
        let expected_state = client.oauth.state.clone();

        // Erst lauschen, dann den Browser öffnen – so geht die Weiterleitung nie ins Leere.
        let listener = TcpListener::bind(("127.0.0.1", REDIRECT_PORT))
            .map_err(|e| AppError::Spotify(format!("Port {REDIRECT_PORT} ist belegt ({e}).")))?;
        open::that(&auth_url)
            .map_err(|e| AppError::Spotify(format!("Browser konnte nicht geöffnet werden: {e}")))?;

        let code = wait_for_code(listener, &expected_state)?;
        client.request_token(&code)?;

        self.db.set_setting(SETTING_CLIENT_ID, client_id)?;
        lock(&self.shared).client_id = client_id.to_owned();
        self.client = Some(client);
        Ok(())
    }

    fn logout(&mut self) -> AppResult<()> {
        self.client = None;
        lock(&self.shared).client_id.clear();
        self.db.delete_setting(SETTING_CLIENT_ID)?;
        match std::fs::remove_file(&self.cache_path) {
            Err(err) if err.kind() != ErrorKind::NotFound => Err(err.into()),
            _ => Ok(()),
        }
    }

    fn control(&mut self, command: Command) -> AppResult<()> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| AppError::Spotify("Nicht verbunden.".into()))?;
        match command {
            Command::Toggle => {
                let playing = fetch_playback(client)?.is_some_and(|p| p.is_playing);
                if playing {
                    client.pause_playback(None)?;
                } else {
                    client.resume_playback(None, None)?;
                }
            }
            Command::Next => client.next_track(None)?,
            Command::Previous => client.previous_track(None)?,
            Command::Volume(percent) => client.volume(percent.min(100), None)?,
            Command::Login(_) | Command::Logout | Command::Refresh => {}
        }
        // Spotify braucht einen Moment, bis der neue Zustand abrufbar ist.
        thread::sleep(Duration::from_millis(350));
        Ok(())
    }
}

fn build_client(client_id: &str, cache_path: PathBuf) -> AuthCodePkceSpotify {
    let creds = Credentials::new_pkce(client_id);
    let oauth = OAuth {
        redirect_uri: REDIRECT_URI.to_owned(),
        scopes: scopes!(
            "user-read-playback-state",
            "user-modify-playback-state",
            "user-read-currently-playing"
        ),
        ..Default::default()
    };
    let config = Config {
        token_cached: true,
        token_refreshing: true,
        cache_path,
        ..Default::default()
    };
    AuthCodePkceSpotify::with_config(creds, oauth, config)
}

fn fetch_playback(client: &AuthCodePkceSpotify) -> AppResult<Option<PlaybackInfo>> {
    let types = [AdditionalType::Track, AdditionalType::Episode];
    let Some(context) = client.current_playback(None, Some(&types))? else {
        return Ok(None);
    };

    let mut info = PlaybackInfo {
        is_playing: context.is_playing,
        progress_ms: context.progress.map(|d| d.num_milliseconds()).unwrap_or(0),
        device: context.device.name,
        volume: context.device.volume_percent,
        ..Default::default()
    };
    match context.item {
        Some(PlayableItem::Track(track)) => {
            info.artist = track
                .artists
                .iter()
                .map(|artist| artist.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            info.duration_ms = track.duration.num_milliseconds();
            info.title = track.name;
        }
        Some(PlayableItem::Episode(episode)) => {
            info.duration_ms = episode.duration.num_milliseconds();
            info.artist = episode.show.name;
            info.title = episode.name;
        }
        #[allow(unreachable_patterns)]
        _ => {}
    }
    Ok(Some(info))
}

fn respond(stream: &mut TcpStream, status: &str, message: &str) {
    let body = format!(
        "<!doctype html><html lang=\"de\"><meta charset=\"utf-8\"><title>Toolless</title>\
         <body style=\"font-family:-apple-system,'Segoe UI',sans-serif;text-align:center;padding-top:18vh\">\
         <h2>{message}</h2><p>Du kannst dieses Fenster schließen.</p></body></html>"
    );
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.flush();
}

/// Wartet auf die Weiterleitung von Spotify an `REDIRECT_URI` und liefert den Auth-Code.
fn wait_for_code(listener: TcpListener, expected_state: &str) -> AppResult<String> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + LOGIN_TIMEOUT;

    loop {
        let mut stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(AppError::Spotify(
                        "Zeitüberschreitung bei der Anmeldung.".into(),
                    ));
                }
                thread::sleep(Duration::from_millis(120));
                continue;
            }
            Err(err) => return Err(err.into()),
        };
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;

        // Erste Zeile der Anfrage: `GET /callback?code=…&state=… HTTP/1.1`
        let mut request_line = String::new();
        if BufReader::new(&stream)
            .read_line(&mut request_line)
            .is_err()
        {
            continue;
        }
        let Some(target) = request_line.split_whitespace().nth(1) else {
            continue;
        };
        if !target.starts_with("/callback") {
            respond(&mut stream, "404 Not Found", "Nicht gefunden");
            continue;
        }
        let Ok(url) = Url::parse(&format!("http://127.0.0.1{target}")) else {
            continue;
        };

        let (mut code, mut state, mut error) = (None, None, None);
        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "code" => code = Some(value.into_owned()),
                "state" => state = Some(value.into_owned()),
                "error" => error = Some(value.into_owned()),
                _ => {}
            }
        }

        if let Some(error) = error {
            respond(&mut stream, "400 Bad Request", "Anmeldung abgebrochen");
            return Err(AppError::Spotify(format!("Anmeldung abgelehnt ({error}).")));
        }
        if state.as_deref() != Some(expected_state) {
            respond(&mut stream, "400 Bad Request", "Ungültige Anfrage");
            return Err(AppError::Spotify("Ungültiger State-Parameter.".into()));
        }
        return match code {
            Some(code) => {
                respond(
                    &mut stream,
                    "200 OK",
                    "Toolless ist jetzt mit Spotify verbunden",
                );
                Ok(code)
            }
            None => {
                respond(&mut stream, "400 Bad Request", "Kein Code erhalten");
                Err(AppError::Spotify(
                    "Kein Autorisierungscode erhalten.".into(),
                ))
            }
        };
    }
}

//! Spotify-Steuerung über die Spotify Web API (rspotify, Authorization Code + PKCE).
//!
//! PKCE benötigt kein Client-Secret – es genügt die Client-ID einer eigenen App
//! aus dem Spotify Developer Dashboard. Das Token wird im App-Datenordner
//! zwischengespeichert und automatisch erneuert.

use std::{
    io::{BufRead, BufReader, ErrorKind, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use rspotify::{
    model::{AdditionalType, PlayableItem},
    prelude::*,
    scopes, AuthCodePkceSpotify, Config, Credentials, OAuth,
};
use serde::Serialize;
use tauri::{async_runtime::Mutex, AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;
use url::Url;

use crate::{
    db::Db,
    error::{AppError, AppResult},
};

const REDIRECT_PORT: u16 = 8888;
/// Muss exakt so im Spotify Developer Dashboard als Redirect URI eingetragen sein.
const REDIRECT_URI: &str = "http://127.0.0.1:8888/callback";
const SETTING_CLIENT_ID: &str = "spotify_client_id";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Default)]
pub struct SpotifyState {
    client: Mutex<Option<AuthCodePkceSpotify>>,
}

impl SpotifyState {
    /// Liefert eine (günstige) Kopie des Clients; das Token wird intern geteilt.
    async fn client(&self) -> AppResult<AuthCodePkceSpotify> {
        self.client
            .lock()
            .await
            .clone()
            .ok_or_else(|| AppError::Spotify("Nicht verbunden.".into()))
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpotifyStatus {
    authenticated: bool,
    client_id: Option<String>,
    redirect_uri: &'static str,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackInfo {
    is_playing: bool,
    title: String,
    artist: String,
    album: String,
    image_url: Option<String>,
    progress_ms: i64,
    duration_ms: i64,
    device: String,
    volume: Option<u32>,
}

fn token_cache_path(app: &AppHandle) -> AppResult<PathBuf> {
    Ok(app.path().app_data_dir()?.join("spotify_token.json"))
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

/// Stellt beim App-Start eine frühere Anmeldung aus dem Token-Cache wieder her.
pub async fn restore(app: &AppHandle) {
    let Ok(Some(client_id)) = app.state::<Db>().get_setting(SETTING_CLIENT_ID) else {
        return;
    };
    let Ok(cache_path) = token_cache_path(app) else {
        return;
    };

    let client = build_client(&client_id, cache_path);
    // Abgelaufene Tokens sind in Ordnung – sie werden beim ersten Aufruf erneuert.
    if let Ok(Some(token)) = client.read_token_cache(true).await {
        if let Ok(mut slot) = client.token.lock().await {
            *slot = Some(token);
        }
        *app.state::<SpotifyState>().client.lock().await = Some(client);
    }
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
                    "Toolless ist jetzt mit Spotify verbunden ✓",
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

async fn fetch_playback(client: &AuthCodePkceSpotify) -> AppResult<Option<PlaybackInfo>> {
    let types = [AdditionalType::Track, AdditionalType::Episode];
    let Some(context) = client.current_playback(None, Some(&types)).await? else {
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
            info.image_url = track.album.images.first().map(|image| image.url.clone());
            info.duration_ms = track.duration.num_milliseconds();
            info.album = track.album.name;
            info.title = track.name;
        }
        Some(PlayableItem::Episode(episode)) => {
            info.image_url = episode.images.first().map(|image| image.url.clone());
            info.duration_ms = episode.duration.num_milliseconds();
            info.artist = episode.show.name;
            info.title = episode.name;
        }
        #[allow(unreachable_patterns)]
        _ => {}
    }
    Ok(Some(info))
}

/// Play/Pause umschalten (auch vom Tray-Menü genutzt).
pub async fn toggle(state: &SpotifyState) -> AppResult<()> {
    let client = state.client().await?;
    let playing = fetch_playback(&client).await?.is_some_and(|p| p.is_playing);
    if playing {
        client.pause_playback(None).await?;
    } else {
        client.resume_playback(None, None).await?;
    }
    Ok(())
}

pub async fn next(state: &SpotifyState) -> AppResult<()> {
    state.client().await?.next_track(None).await?;
    Ok(())
}

pub async fn previous(state: &SpotifyState) -> AppResult<()> {
    state.client().await?.previous_track(None).await?;
    Ok(())
}

#[tauri::command]
pub async fn spotify_status(
    db: State<'_, Db>,
    state: State<'_, SpotifyState>,
) -> AppResult<SpotifyStatus> {
    Ok(SpotifyStatus {
        authenticated: state.client.lock().await.is_some(),
        client_id: db.get_setting(SETTING_CLIENT_ID)?,
        redirect_uri: REDIRECT_URI,
    })
}

/// Startet den PKCE-Login: öffnet den Browser und wartet lokal auf die Weiterleitung.
#[tauri::command]
pub async fn spotify_login(
    app: AppHandle,
    db: State<'_, Db>,
    state: State<'_, SpotifyState>,
    client_id: String,
) -> AppResult<()> {
    let client_id = client_id.trim().to_owned();
    if client_id.is_empty() {
        return Err(AppError::Spotify("Bitte eine Client-ID eintragen.".into()));
    }

    let mut client = build_client(&client_id, token_cache_path(&app)?);
    let auth_url = client.get_authorize_url(None)?;
    let expected_state = client.oauth.state.clone();

    // Erst lauschen, dann den Browser öffnen – so geht die Weiterleitung nie ins Leere.
    let listener = TcpListener::bind(("127.0.0.1", REDIRECT_PORT)).map_err(|e| {
        AppError::Spotify(format!(
            "Port {REDIRECT_PORT} ist belegt ({e}). Läuft bereits eine Anmeldung?"
        ))
    })?;
    app.opener()
        .open_url(auth_url, None::<&str>)
        .map_err(|e| AppError::Spotify(format!("Browser konnte nicht geöffnet werden: {e}")))?;

    let code =
        tauri::async_runtime::spawn_blocking(move || wait_for_code(listener, &expected_state))
            .await
            .map_err(|e| AppError::Spotify(e.to_string()))??;

    client.request_token(&code).await?;
    db.set_setting(SETTING_CLIENT_ID, &client_id)?;
    *state.client.lock().await = Some(client);
    Ok(())
}

#[tauri::command]
pub async fn spotify_logout(
    app: AppHandle,
    db: State<'_, Db>,
    state: State<'_, SpotifyState>,
) -> AppResult<()> {
    *state.client.lock().await = None;
    db.delete_setting(SETTING_CLIENT_ID)?;
    match std::fs::remove_file(token_cache_path(&app)?) {
        Err(err) if err.kind() != ErrorKind::NotFound => Err(err.into()),
        _ => Ok(()),
    }
}

#[tauri::command]
pub async fn spotify_playback(state: State<'_, SpotifyState>) -> AppResult<Option<PlaybackInfo>> {
    fetch_playback(&state.client().await?).await
}

#[tauri::command]
pub async fn spotify_toggle(state: State<'_, SpotifyState>) -> AppResult<()> {
    toggle(&state).await
}

#[tauri::command]
pub async fn spotify_next(state: State<'_, SpotifyState>) -> AppResult<()> {
    next(&state).await
}

#[tauri::command]
pub async fn spotify_previous(state: State<'_, SpotifyState>) -> AppResult<()> {
    previous(&state).await
}

#[tauri::command]
pub async fn spotify_set_volume(state: State<'_, SpotifyState>, percent: u8) -> AppResult<()> {
    state.client().await?.volume(percent.min(100), None).await?;
    Ok(())
}

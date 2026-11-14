import { useCallback, useEffect, useState } from "react";
import { Icon } from "../components/Icon";
import { api, type Playback, type SpotifyStatus } from "../lib/api";
import type { ModuleProps } from "../types";

const POLL_MS = 3000;

function formatTime(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

export default function Spotify({ notify }: ModuleProps) {
  const [status, setStatus] = useState<SpotifyStatus | null>(null);
  const [playback, setPlayback] = useState<Playback | null>(null);
  const [clientId, setClientId] = useState("");
  const [connecting, setConnecting] = useState(false);
  /** Lautstärke während des Ziehens – verhindert Springen durch das Polling. */
  const [volumeDraft, setVolumeDraft] = useState<number | null>(null);

  const loadStatus = useCallback(async () => {
    try {
      const next = await api.spotifyStatus();
      setStatus(next);
      setClientId((current) => current || next.clientId || "");
    } catch (err) {
      notify(err);
    }
  }, [notify]);

  const refresh = useCallback(async () => {
    try {
      setPlayback(await api.spotifyPlayback());
    } catch {
      // Beim Polling still bleiben – Fehler zeigen sich bei der nächsten Aktion.
    }
  }, []);

  useEffect(() => void loadStatus(), [loadStatus]);

  const authenticated = status?.authenticated ?? false;
  useEffect(() => {
    if (!authenticated) return;
    void refresh();
    const timer = window.setInterval(() => void refresh(), POLL_MS);
    return () => window.clearInterval(timer);
  }, [authenticated, refresh]);

  async function connect() {
    setConnecting(true);
    try {
      await api.spotifyLogin(clientId);
      await loadStatus();
    } catch (err) {
      notify(err);
    } finally {
      setConnecting(false);
    }
  }

  async function disconnect() {
    try {
      await api.spotifyLogout();
      setPlayback(null);
      await loadStatus();
    } catch (err) {
      notify(err);
    }
  }

  /** Führt eine Steueraktion aus und holt kurz danach den neuen Zustand. */
  async function control(action: () => Promise<void>) {
    try {
      await action();
      window.setTimeout(() => void refresh(), 350);
    } catch (err) {
      notify(err);
    }
  }

  function commitVolume() {
    if (volumeDraft == null) return;
    const value = volumeDraft;
    void control(() => api.spotifySetVolume(value)).finally(() => window.setTimeout(() => setVolumeDraft(null), 800));
  }

  if (!status) return null;

  if (!authenticated) {
    return (
      <div className="stack narrow">
        <div className="card">
          <div className="card-title">Mit Spotify verbinden</div>
          <ol className="steps">
            <li>
              Lege im Spotify Developer Dashboard (developer.spotify.com/dashboard) eine App an und wähle „Web API“.
            </li>
            <li>
              Trage dort als Redirect URI exakt <code>{status.redirectUri}</code> ein.
            </li>
            <li>Kopiere die Client-ID hierher und verbinde dich. Ein Client-Secret wird nicht benötigt.</li>
          </ol>
          <div className="form-row">
            <input
              className="input"
              placeholder="Client-ID"
              spellCheck={false}
              value={clientId}
              onChange={(e) => setClientId(e.target.value)}
            />
            <button className="btn primary" disabled={!clientId.trim() || connecting} onClick={() => void connect()}>
              {connecting ? "Warte auf Browser …" : "Verbinden"}
            </button>
          </div>
          <p className="muted small">
            Die Wiedergabesteuerung über die Web API setzt Spotify Premium und ein aktives Gerät voraus.
          </p>
        </div>
      </div>
    );
  }

  const volume = volumeDraft ?? playback?.volume ?? 0;
  const progress = playback && playback.durationMs > 0 ? (playback.progressMs / playback.durationMs) * 100 : 0;

  return (
    <div className="stack narrow">
      <div className="card player">
        {playback?.imageUrl ? (
          <img className="artwork" src={playback.imageUrl} alt={`Cover: ${playback.album || playback.title}`} />
        ) : (
          <div className="artwork placeholder">
            <Icon name="music" size={44} />
          </div>
        )}

        <div className="player-meta">
          <div className="player-title">{playback?.title || "Keine Wiedergabe"}</div>
          <div className="muted">
            {playback ? playback.artist : "Starte die Wiedergabe in Spotify, um sie hier zu steuern."}
          </div>
        </div>

        <div className="progress" aria-hidden="true">
          <div className="progress-fill" style={{ width: `${progress}%` }} />
        </div>
        <div className="progress-times muted small">
          <span>{formatTime(playback?.progressMs ?? 0)}</span>
          <span>{formatTime(playback?.durationMs ?? 0)}</span>
        </div>

        <div className="player-controls">
          <button
            className="icon-btn large"
            aria-label="Vorheriger Titel"
            onClick={() => void control(api.spotifyPrevious)}
          >
            <Icon name="previous" size={24} />
          </button>
          <button
            className="play-btn"
            aria-label={playback?.isPlaying ? "Pause" : "Wiedergabe"}
            onClick={() => void control(api.spotifyToggle)}
          >
            <Icon name={playback?.isPlaying ? "pause" : "play"} size={26} />
          </button>
          <button className="icon-btn large" aria-label="Nächster Titel" onClick={() => void control(api.spotifyNext)}>
            <Icon name="next" size={24} />
          </button>
        </div>

        {playback?.volume != null && (
          <label className="volume">
            <span className="muted small">Lautstärke</span>
            <input
              type="range"
              min={0}
              max={100}
              value={volume}
              onChange={(e) => setVolumeDraft(Number(e.target.value))}
              onPointerUp={commitVolume}
              onKeyUp={commitVolume}
            />
            <span className="muted small volume-value">{volume} %</span>
          </label>
        )}
      </div>

      <div className="group">
        <div className="group-row">
          <div>
            <div className="row-title">{playback?.device || "Kein aktives Gerät"}</div>
            <div className="row-sub">Wiedergabegerät</div>
          </div>
          <button className="btn" onClick={() => void disconnect()}>
            Trennen
          </button>
        </div>
      </div>
    </div>
  );
}

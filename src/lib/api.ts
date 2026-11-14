// Typisierte Brücke zu den Rust-Commands (src-tauri/src/commands/*).
import { invoke } from "@tauri-apps/api/core";

export interface ClipItem {
  id: number;
  title: string;
  category: string;
  content: string;
  createdAt: string;
}

export interface AppEntry {
  name: string;
  path: string;
}

export interface LaunchResult {
  path: string;
  ok: boolean;
  error: string | null;
}

export interface SpotifyStatus {
  authenticated: boolean;
  clientId: string | null;
  redirectUri: string;
}

export interface Playback {
  isPlaying: boolean;
  title: string;
  artist: string;
  album: string;
  imageUrl: string | null;
  progressMs: number;
  durationMs: number;
  device: string;
  volume: number | null;
}

export const EARWEB_URL = "https://earweb.bttl.xyz/index.html";
export const KEEP_AWAKE_EVENT = "keep-awake-changed";

export const api = {
  // Zwischenablage
  clipboardRead: () => invoke<string>("clipboard_read"),
  clipboardWrite: (text: string) => invoke<void>("clipboard_write", { text }),
  clipList: () => invoke<ClipItem[]>("clip_list"),
  clipSave: (title: string, category: string, content?: string) =>
    invoke<ClipItem>("clip_save", { title, category, content: content ?? null }),
  clipUpdate: (item: Pick<ClipItem, "id" | "title" | "category" | "content">) => invoke<ClipItem>("clip_update", item),
  clipDelete: (id: number) => invoke<void>("clip_delete", { id }),
  clipCopy: (id: number) => invoke<void>("clip_copy", { id }),

  // Autostart & Ruhezustand
  autostartIsEnabled: () => invoke<boolean>("autostart_is_enabled"),
  autostartSet: (enabled: boolean) => invoke<boolean>("autostart_set", { enabled }),
  powerIsActive: () => invoke<boolean>("power_is_active"),
  powerSet: (enabled: boolean) => invoke<boolean>("power_set", { enabled }),

  // App-Starter
  launcherScan: () => invoke<AppEntry[]>("launcher_scan"),
  launcherLaunch: (paths: string[]) => invoke<LaunchResult[]>("launcher_launch", { paths }),

  // Spotify
  spotifyStatus: () => invoke<SpotifyStatus>("spotify_status"),
  spotifyLogin: (clientId: string) => invoke<void>("spotify_login", { clientId }),
  spotifyLogout: () => invoke<void>("spotify_logout"),
  spotifyPlayback: () => invoke<Playback | null>("spotify_playback"),
  spotifyToggle: () => invoke<void>("spotify_toggle"),
  spotifyNext: () => invoke<void>("spotify_next"),
  spotifyPrevious: () => invoke<void>("spotify_previous"),
  spotifySetVolume: (percent: number) => invoke<void>("spotify_set_volume", { percent }),

  // Web
  openEarwebWindow: () => invoke<void>("open_earweb_window"),
};

/** Macht aus beliebigen Fehlerwerten eine anzeigbare Meldung. */
export function errorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return JSON.stringify(err);
}

import { useCallback, useEffect, useMemo, useState } from "react";
import { Icon } from "../components/Icon";
import { api, type AppEntry } from "../lib/api";
import type { ModuleProps } from "../types";

const STORAGE_KEY = "toolless.launcher.selection";

function loadSelection(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "[]") as string[]);
  } catch {
    return new Set();
  }
}

export default function Launcher({ notify }: ModuleProps) {
  const [apps, setApps] = useState<AppEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState("");
  const [onlySelected, setOnlySelected] = useState(false);
  // Die Auswahl bleibt erhalten – so lässt sich dasselbe App-Set jeden Tag mit einem Klick starten.
  const [selected, setSelected] = useState<Set<string>>(loadSelection);

  const scan = useCallback(() => {
    setLoading(true);
    api
      .launcherScan()
      .then(setApps)
      .catch(notify)
      .finally(() => setLoading(false));
  }, [notify]);

  useEffect(scan, [scan]);

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify([...selected]));
    } catch {
      /* Speicher nicht verfügbar */
    }
  }, [selected]);

  const visible = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return apps.filter(
      (app) => (!needle || app.name.toLowerCase().includes(needle)) && (!onlySelected || selected.has(app.path)),
    );
  }, [apps, search, onlySelected, selected]);

  // Nur zählen, was es nach dem letzten Scan noch gibt.
  const selectedPaths = useMemo(
    () => apps.filter((app) => selected.has(app.path)).map((app) => app.path),
    [apps, selected],
  );

  function toggle(path: string) {
    setSelected((current) => {
      const next = new Set(current);
      if (!next.delete(path)) next.add(path);
      return next;
    });
  }

  async function launch(paths: string[]) {
    if (paths.length === 0) return;
    try {
      const results = await api.launcherLaunch(paths);
      const failed = results.filter((result) => !result.ok);
      notify(
        failed.length === 0
          ? `${results.length} ${results.length === 1 ? "App" : "Apps"} gestartet`
          : `${failed.length} von ${results.length} fehlgeschlagen: ${failed[0].error ?? ""}`,
      );
    } catch (err) {
      notify(err);
    }
  }

  return (
    <div className="stack">
      <div className="filter-bar">
        <label className="search grow">
          <Icon name="search" size={15} />
          <input placeholder="Apps durchsuchen" value={search} onChange={(e) => setSearch(e.target.value)} />
        </label>
        <button className={`chip${onlySelected ? " active" : ""}`} onClick={() => setOnlySelected((v) => !v)}>
          Auswahl ({selectedPaths.length})
        </button>
        <button className="btn ghost" onClick={scan} disabled={loading}>
          <Icon name="refresh" size={15} /> Neu scannen
        </button>
        <button
          className="btn primary"
          disabled={selectedPaths.length === 0}
          onClick={() => void launch(selectedPaths)}
        >
          <Icon name="play" size={14} /> Ausgewählte starten
        </button>
      </div>

      {loading ? (
        <p className="empty-state">Programme werden gesucht …</p>
      ) : visible.length === 0 ? (
        <p className="empty-state">Keine Programme gefunden.</p>
      ) : (
        <ul className="app-grid">
          {visible.map((app) => {
            const isSelected = selected.has(app.path);
            return (
              <li key={app.path}>
                <div className={`app-tile${isSelected ? " selected" : ""}`}>
                  <label className="app-check">
                    <input
                      type="checkbox"
                      checked={isSelected}
                      onChange={() => toggle(app.path)}
                      aria-label={`${app.name} auswählen`}
                    />
                  </label>
                  <button className="app-main" title={`${app.name} starten`} onClick={() => void launch([app.path])}>
                    <span className="app-avatar" aria-hidden="true">
                      {app.name.charAt(0).toUpperCase()}
                    </span>
                    <span className="app-name">{app.name}</span>
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

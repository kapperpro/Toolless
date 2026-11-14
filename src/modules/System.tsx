import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Toggle } from "../components/Toggle";
import { api, KEEP_AWAKE_EVENT } from "../lib/api";
import type { ModuleProps } from "../types";

export default function System({ notify }: ModuleProps) {
  const [autostart, setAutostart] = useState(false);
  const [keepAwake, setKeepAwake] = useState(false);

  useEffect(() => {
    api.autostartIsEnabled().then(setAutostart).catch(notify);
    api.powerIsActive().then(setKeepAwake).catch(notify);
    // Das Tray-Menü kann den Zustand ebenfalls umschalten.
    const unlisten = listen<boolean>(KEEP_AWAKE_EVENT, (event) => setKeepAwake(event.payload));
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, [notify]);

  return (
    <div className="stack narrow">
      <div className="group">
        <div className="group-row">
          <div>
            <div className="row-title">Ruhezustand blockieren</div>
            <div className="row-sub wrap">
              Verhindert Standby und Bildschirm-Aus, solange der Schalter aktiv ist. Endet automatisch mit der App.
            </div>
          </div>
          <Toggle
            label="Ruhezustand blockieren"
            checked={keepAwake}
            onChange={(value) => api.powerSet(value).then(setKeepAwake).catch(notify)}
          />
        </div>
        <div className="group-row">
          <div>
            <div className="row-title">Mit dem System starten</div>
            <div className="row-sub wrap">
              Trägt Toolless in den Autostart ein. Die App startet dann unauffällig im System-Tray.
            </div>
          </div>
          <Toggle
            label="Mit dem System starten"
            checked={autostart}
            onChange={(value) => api.autostartSet(value).then(setAutostart).catch(notify)}
          />
        </div>
      </div>

      <div className="group">
        <div className="group-row">
          <div>
            <div className="row-title">Hintergrundbetrieb</div>
            <div className="row-sub wrap">
              Schließen versteckt nur das Fenster. Über das Tray-Icon öffnest du Toolless wieder, steuerst Spotify oder
              beendest die App vollständig.
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

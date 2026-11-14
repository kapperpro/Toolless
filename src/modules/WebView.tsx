import { useState } from "react";
import { Icon } from "../components/Icon";
import { api, EARWEB_URL } from "../lib/api";
import type { ModuleProps } from "../types";

/** EarWeb direkt im Tab (iFrame) – alternativ als eigenes natives Fenster. */
export default function WebView({ notify }: ModuleProps) {
  const [reloadKey, setReloadKey] = useState(0);

  return (
    <div className="web">
      <div className="web-bar">
        <span className="web-url">{EARWEB_URL}</span>
        <button className="btn ghost" onClick={() => setReloadKey((key) => key + 1)}>
          <Icon name="refresh" size={15} /> Neu laden
        </button>
        <button className="btn ghost" onClick={() => api.openEarwebWindow().catch(notify)}>
          <Icon name="external" size={15} /> Eigenes Fenster
        </button>
      </div>
      <iframe
        key={reloadKey}
        className="web-frame"
        src={EARWEB_URL}
        title="EarWeb"
        allow="bluetooth; serial; usb; hid; clipboard-write"
        referrerPolicy="no-referrer"
      />
    </div>
  );
}

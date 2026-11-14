// „Ampel“-Buttons wie unter macOS. Schließen versteckt das Fenster nur –
// die App läuft im System-Tray weiter (siehe src-tauri/src/lib.rs).
import { getCurrentWindow } from "@tauri-apps/api/window";

export function WindowControls() {
  const win = getCurrentWindow();
  return (
    <div className="traffic-lights">
      <button className="light close" aria-label="Schließen (läuft im Tray weiter)" onClick={() => void win.close()} />
      <button className="light minimize" aria-label="Minimieren" onClick={() => void win.minimize()} />
      <button className="light zoom" aria-label="Zoomen" onClick={() => void win.toggleMaximize()} />
    </div>
  );
}

import { useCallback, useEffect, useRef, useState } from "react";
import { Sidebar, type NavItem } from "./components/Sidebar";
import { errorMessage } from "./lib/api";
import Calculator from "./modules/Calculator";
import ClipboardManager from "./modules/ClipboardManager";
import Launcher from "./modules/Launcher";
import Notes from "./modules/Notes";
import Spotify from "./modules/Spotify";
import System from "./modules/System";
import WebView from "./modules/WebView";
import type { ModuleProps } from "./types";

const NAV = [
  { id: "spotify", label: "Spotify", icon: "music" },
  { id: "clipboard", label: "Zwischenablage", icon: "clipboard" },
  { id: "notes", label: "Notizen", icon: "note" },
  { id: "calculator", label: "Rechner", icon: "calculator" },
  { id: "launcher", label: "App-Starter", icon: "grid" },
  { id: "web", label: "EarWeb", icon: "globe" },
  { id: "system", label: "System", icon: "gear" },
] as const satisfies readonly NavItem<string>[];

type ModuleId = (typeof NAV)[number]["id"];

const MODULES: Record<ModuleId, (props: ModuleProps) => JSX.Element | null> = {
  spotify: Spotify,
  clipboard: ClipboardManager,
  notes: Notes,
  calculator: Calculator,
  launcher: Launcher,
  web: WebView,
  system: System,
};

const STORAGE_KEY = "toolless.activeModule";

function initialModule(): ModuleId {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored && stored in MODULES) return stored as ModuleId;
  } catch {
    /* Speicher nicht verfügbar */
  }
  return "spotify";
}

export default function App() {
  const [active, setActive] = useState<ModuleId>(initialModule);
  const [toast, setToast] = useState<string | null>(null);
  const timer = useRef<number>();

  const notify = useCallback((message: unknown) => {
    setToast(errorMessage(message));
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setToast(null), 3800);
  }, []);

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, active);
    } catch {
      /* Speicher nicht verfügbar */
    }
  }, [active]);

  const Module = MODULES[active];
  const title = NAV.find((item) => item.id === active)?.label ?? "";

  return (
    <div className="window">
      <Sidebar items={NAV} active={active} onSelect={setActive} />
      <main className="content">
        <header className="toolbar" data-tauri-drag-region>
          <h1>{title}</h1>
        </header>
        <section className={`view view-${active}`}>
          <Module notify={notify} />
        </section>
      </main>
      {toast && (
        <div className="toast" role="status">
          {toast}
        </div>
      )}
    </div>
  );
}

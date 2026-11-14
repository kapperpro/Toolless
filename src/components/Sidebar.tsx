import { Icon, type IconName } from "./Icon";
import { WindowControls } from "./WindowControls";

export interface NavItem<Id extends string> {
  id: Id;
  label: string;
  icon: IconName;
}

interface SidebarProps<Id extends string> {
  items: readonly NavItem<Id>[];
  active: Id;
  onSelect: (id: Id) => void;
}

export function Sidebar<Id extends string>({ items, active, onSelect }: SidebarProps<Id>) {
  return (
    <aside className="sidebar">
      <div className="sidebar-top" data-tauri-drag-region>
        <WindowControls />
      </div>
      <nav className="sidebar-nav" aria-label="Module">
        <div className="sidebar-heading">Werkzeuge</div>
        {items.map((item) => (
          <button
            key={item.id}
            className={`nav-item${item.id === active ? " active" : ""}`}
            aria-current={item.id === active ? "page" : undefined}
            onClick={() => onSelect(item.id)}
          >
            <Icon name={item.icon} />
            <span>{item.label}</span>
          </button>
        ))}
      </nav>
      <div className="sidebar-footer">Toolless läuft im Tray weiter, wenn du das Fenster schließt.</div>
    </aside>
  );
}

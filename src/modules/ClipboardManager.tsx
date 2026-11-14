import { useCallback, useEffect, useMemo, useState } from "react";
import { Icon } from "../components/Icon";
import { api, type ClipItem } from "../lib/api";
import type { ModuleProps } from "../types";

const ALL = "__all__";

export default function ClipboardManager({ notify }: ModuleProps) {
  const [items, setItems] = useState<ClipItem[]>([]);
  const [current, setCurrent] = useState("");
  const [title, setTitle] = useState("");
  const [category, setCategory] = useState("");
  const [filter, setFilter] = useState(ALL);
  const [search, setSearch] = useState("");
  const [editing, setEditing] = useState<ClipItem | null>(null);
  const [copiedId, setCopiedId] = useState<number | null>(null);

  const readClipboard = useCallback(() => {
    api.clipboardRead().then(setCurrent).catch(notify);
  }, [notify]);

  useEffect(() => {
    api.clipList().then(setItems).catch(notify);
    readClipboard();
    // Beim Zurückwechseln in die App den aktuellen Inhalt neu einlesen.
    window.addEventListener("focus", readClipboard);
    return () => window.removeEventListener("focus", readClipboard);
  }, [notify, readClipboard]);

  const categories = useMemo(
    () => [...new Set(items.map((item) => item.category).filter(Boolean))].sort((a, b) => a.localeCompare(b)),
    [items],
  );

  const visible = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return items.filter(
      (item) =>
        (filter === ALL || item.category === filter) &&
        (!needle || `${item.title}\n${item.content}\n${item.category}`.toLowerCase().includes(needle)),
    );
  }, [items, filter, search]);

  async function save() {
    try {
      const item = await api.clipSave(title, category, current);
      setItems((list) => [item, ...list]);
      setTitle("");
      notify(`„${item.title}“ gespeichert`);
    } catch (err) {
      notify(err);
    }
  }

  async function copy(item: ClipItem) {
    try {
      await api.clipCopy(item.id);
      setCurrent(item.content);
      setCopiedId(item.id);
      window.setTimeout(() => setCopiedId((id) => (id === item.id ? null : id)), 1400);
    } catch (err) {
      notify(err);
    }
  }

  async function remove(item: ClipItem) {
    try {
      await api.clipDelete(item.id);
      setItems((list) => list.filter((entry) => entry.id !== item.id));
    } catch (err) {
      notify(err);
    }
  }

  async function saveEdit() {
    if (!editing) return;
    try {
      const updated = await api.clipUpdate({
        id: editing.id,
        title: editing.title,
        category: editing.category,
        content: editing.content,
      });
      setItems((list) => list.map((entry) => (entry.id === updated.id ? updated : entry)));
      setEditing(null);
    } catch (err) {
      notify(err);
    }
  }

  return (
    <div className="stack">
      <div className="card">
        <div className="card-header">
          <div className="card-title">Aktuelle Zwischenablage</div>
          <button className="btn ghost" onClick={readClipboard}>
            <Icon name="refresh" size={15} /> Neu einlesen
          </button>
        </div>
        <pre className={`clip-preview${current ? "" : " empty"}`}>
          {current || "Die Zwischenablage enthält gerade keinen Text."}
        </pre>
        <div className="form-row">
          <input
            className="input"
            placeholder="Titel (optional)"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && current && void save()}
          />
          <input
            className="input"
            placeholder="Kategorie"
            list="clip-categories"
            value={category}
            onChange={(e) => setCategory(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && current && void save()}
          />
          <datalist id="clip-categories">
            {categories.map((name) => (
              <option key={name} value={name} />
            ))}
          </datalist>
          <button className="btn primary" disabled={!current} onClick={() => void save()}>
            Speichern
          </button>
        </div>
      </div>

      <div className="filter-bar">
        <div className="chips" role="tablist" aria-label="Kategorien">
          <button className={`chip${filter === ALL ? " active" : ""}`} onClick={() => setFilter(ALL)}>
            Alle
          </button>
          {categories.map((name) => (
            <button key={name} className={`chip${filter === name ? " active" : ""}`} onClick={() => setFilter(name)}>
              {name}
            </button>
          ))}
        </div>
        <label className="search">
          <Icon name="search" size={15} />
          <input placeholder="Suchen" value={search} onChange={(e) => setSearch(e.target.value)} />
        </label>
      </div>

      {visible.length === 0 ? (
        <p className="empty-state">
          {items.length === 0
            ? "Noch nichts gespeichert. Kopiere etwas und sichere es oben mit Titel und Kategorie."
            : "Keine Einträge für diesen Filter."}
        </p>
      ) : (
        <ul className="list">
          {visible.map((item) =>
            editing?.id === item.id ? (
              <li key={item.id} className="list-row editing">
                <div className="form-row">
                  <input
                    className="input"
                    aria-label="Titel"
                    value={editing.title}
                    onChange={(e) => setEditing({ ...editing, title: e.target.value })}
                  />
                  <input
                    className="input"
                    aria-label="Kategorie"
                    list="clip-categories"
                    value={editing.category}
                    onChange={(e) => setEditing({ ...editing, category: e.target.value })}
                  />
                </div>
                <textarea
                  className="input"
                  aria-label="Inhalt"
                  rows={4}
                  value={editing.content}
                  onChange={(e) => setEditing({ ...editing, content: e.target.value })}
                />
                <div className="row-actions end">
                  <button className="btn" onClick={() => setEditing(null)}>
                    Abbrechen
                  </button>
                  <button className="btn primary" onClick={() => void saveEdit()}>
                    Sichern
                  </button>
                </div>
              </li>
            ) : (
              <li key={item.id} className="list-row">
                <button className="row-main" title="In die Zwischenablage kopieren" onClick={() => void copy(item)}>
                  <span className="row-title">
                    {item.title}
                    {item.category && <span className="badge">{item.category}</span>}
                  </span>
                  <span className="row-sub">{item.content}</span>
                </button>
                <div className="row-actions">
                  <button className="icon-btn" aria-label="Kopieren" onClick={() => void copy(item)}>
                    <Icon name={copiedId === item.id ? "check" : "copy"} />
                  </button>
                  <button className="icon-btn" aria-label="Bearbeiten" onClick={() => setEditing(item)}>
                    <Icon name="pencil" />
                  </button>
                  <button className="icon-btn danger" aria-label="Löschen" onClick={() => void remove(item)}>
                    <Icon name="trash" />
                  </button>
                </div>
              </li>
            ),
          )}
        </ul>
      )}
    </div>
  );
}

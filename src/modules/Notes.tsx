import { useCallback, useEffect, useRef, useState } from "react";
import { Icon } from "../components/Icon";
import { createNote, deleteNote, listNotes, saveNote, type Note } from "../lib/notes";
import type { ModuleProps } from "../types";

const SAVE_DELAY_MS = 500;

export default function Notes({ notify }: ModuleProps) {
  const [notes, setNotes] = useState<Note[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  /** Ausstehende (verzögerte) Speichervorgänge je Notiz. */
  const pending = useRef(new Map<string, { timer: number; note: Note }>());

  useEffect(() => {
    listNotes()
      .then((list) => {
        setNotes(list);
        setActiveId(list[0]?.id ?? null);
      })
      .catch(notify)
      .finally(() => setLoaded(true));
  }, [notify]);

  // Beim Verlassen des Moduls alles Ausstehende sofort schreiben.
  useEffect(() => {
    const queue = pending.current;
    return () => {
      for (const { timer, note } of queue.values()) {
        window.clearTimeout(timer);
        void saveNote(note);
      }
      queue.clear();
    };
  }, []);

  const scheduleSave = useCallback(
    (note: Note) => {
      const queue = pending.current;
      window.clearTimeout(queue.get(note.id)?.timer);
      const timer = window.setTimeout(() => {
        queue.delete(note.id);
        saveNote(note).catch(notify);
      }, SAVE_DELAY_MS);
      queue.set(note.id, { timer, note });
    },
    [notify],
  );

  const active = notes.find((note) => note.id === activeId) ?? null;

  function update(patch: Partial<Pick<Note, "title" | "body">>) {
    if (!active) return;
    const next: Note = { ...active, ...patch, updatedAt: Date.now() };
    setNotes((list) => [next, ...list.filter((note) => note.id !== next.id)]);
    scheduleSave(next);
  }

  async function add() {
    const note = createNote();
    try {
      await saveNote(note);
      setNotes((list) => [note, ...list]);
      setActiveId(note.id);
    } catch (err) {
      notify(err);
    }
  }

  async function removeActive() {
    if (!active) return;
    const queue = pending.current;
    window.clearTimeout(queue.get(active.id)?.timer);
    queue.delete(active.id);
    try {
      await deleteNote(active.id);
      const rest = notes.filter((note) => note.id !== active.id);
      setNotes(rest);
      setActiveId(rest[0]?.id ?? null);
    } catch (err) {
      notify(err);
    }
  }

  return (
    <div className="notes">
      <div className="notes-list">
        <div className="notes-list-header">
          <span className="muted">{notes.length} Notizen</span>
          <button className="icon-btn" aria-label="Neue Notiz" onClick={() => void add()}>
            <Icon name="plus" />
          </button>
        </div>
        <ul className="plain-list">
          {notes.map((note) => (
            <li key={note.id}>
              <button
                className={`note-item${note.id === activeId ? " active" : ""}`}
                onClick={() => setActiveId(note.id)}
              >
                <span className="row-title">{note.title || "Neue Notiz"}</span>
                <span className="row-sub">
                  {new Date(note.updatedAt).toLocaleDateString("de-DE", {
                    day: "2-digit",
                    month: "2-digit",
                    year: "2-digit",
                  })}{" "}
                  {note.body.split("\n")[0] || "Kein weiterer Text"}
                </span>
              </button>
            </li>
          ))}
        </ul>
      </div>

      {active ? (
        <div className="notes-editor">
          <div className="notes-editor-header">
            <input
              className="note-title"
              placeholder="Titel"
              value={active.title}
              onChange={(e) => update({ title: e.target.value })}
            />
            <button className="icon-btn danger" aria-label="Notiz löschen" onClick={() => void removeActive()}>
              <Icon name="trash" />
            </button>
          </div>
          <textarea
            className="note-body"
            placeholder="Schreib los …"
            value={active.body}
            onChange={(e) => update({ body: e.target.value })}
          />
        </div>
      ) : (
        <div className="notes-editor placeholder">
          {loaded && (
            <>
              <p className="muted">Keine Notiz ausgewählt.</p>
              <button className="btn primary" onClick={() => void add()}>
                Neue Notiz
              </button>
            </>
          )}
        </div>
      )}
    </div>
  );
}

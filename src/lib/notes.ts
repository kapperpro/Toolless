// Notizen: CRUD direkt über die Tauri-FS-API.
// Jede Notiz ist eine JSON-Datei unter <App-Datenordner>/notes/<id>.json.
import { BaseDirectory, exists, mkdir, readDir, readTextFile, remove, writeTextFile } from "@tauri-apps/plugin-fs";

export interface Note {
  id: string;
  title: string;
  body: string;
  updatedAt: number;
}

const DIR = "notes";
const base = { baseDir: BaseDirectory.AppData };
const fileOf = (id: string) => `${DIR}/${id}.json`;

async function ensureDir(): Promise<void> {
  if (!(await exists(DIR, base))) {
    await mkdir(DIR, { ...base, recursive: true });
  }
}

export function createNote(): Note {
  return { id: crypto.randomUUID(), title: "", body: "", updatedAt: Date.now() };
}

export async function listNotes(): Promise<Note[]> {
  await ensureDir();
  const notes: Note[] = [];
  for (const entry of await readDir(DIR, base)) {
    if (!entry.isFile || !entry.name.endsWith(".json")) continue;
    try {
      const note = JSON.parse(await readTextFile(`${DIR}/${entry.name}`, base)) as Note;
      if (typeof note.id === "string") notes.push(note);
    } catch {
      // Beschädigte Datei überspringen statt die ganze Liste scheitern zu lassen.
    }
  }
  return notes.sort((a, b) => b.updatedAt - a.updatedAt);
}

export async function saveNote(note: Note): Promise<void> {
  await ensureDir();
  await writeTextFile(fileOf(note.id), JSON.stringify(note, null, 2), base);
}

export async function deleteNote(id: string): Promise<void> {
  if (await exists(fileOf(id), base)) {
    await remove(fileOf(id), base);
  }
}

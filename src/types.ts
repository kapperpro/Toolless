/** Gemeinsame Props aller Module. */
export interface ModuleProps {
  /** Zeigt eine kurze Meldung (Toast) an; Fehlerobjekte werden in Text umgewandelt. */
  notify: (message: unknown) => void;
}

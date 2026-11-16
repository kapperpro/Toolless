//! Sprache der Oberfläche. Die Texte stehen auf Deutsch im Code; für Englisch werden
//! sie beim Anzeigen über die Tabellen in `i18n_table` übersetzt.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
};

use crate::i18n_table::{FRAGMENTS, PHRASES};

static ENGLISH: AtomicBool = AtomicBool::new(false);

pub fn set_english(enabled: bool) {
    ENGLISH.store(enabled, Ordering::Relaxed);
}

pub fn is_english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

fn phrases() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| PHRASES.iter().copied().collect())
}

/// Übersetzt einen festen Text der Oberfläche. Unbekannte Texte bleiben, wie sie sind.
pub fn t(text: &str) -> &str {
    if !is_english() {
        return text;
    }
    phrases().get(text).copied().unwrap_or(text)
}

/// Übersetzt einen zusammengesetzten Text (mit eingesetzten Werten), indem die festen
/// Textteile ersetzt werden. Die Tabelle ist nach Länge sortiert, damit längere Teile
/// vor ihren Bruchstücken greifen.
pub fn td(text: String) -> String {
    if !is_english() {
        return text;
    }
    if let Some(known) = phrases().get(text.as_str()) {
        return (*known).to_owned();
    }
    let mut out = text;
    for (german, english) in FRAGMENTS {
        if out.contains(german) {
            out = out.replace(german, english);
        }
    }
    out
}

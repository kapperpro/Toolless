//! Notiztext mit eingebetteten Bildern.
//!
//! Ein Bild steht im Text als eigene Zeile der Form `![bild](dateiname.png)`.
//! Für den Editor wird der Text in Blöcke zerlegt (Text – Bild – Text …) und
//! nach dem Bearbeiten wieder zusammengesetzt.

const MARKER_START: &str = "![bild](";
const MARKER_END: char = ')';

/// Ein Abschnitt der Notiz.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Text(String),
    /// Dateiname eines Bildes im Bilderordner der Notizen.
    Image(String),
}

/// Die Textzeile, die ein Bild im Notiztext vertritt.
pub fn marker(name: &str) -> String {
    format!("{MARKER_START}{name}{MARKER_END}")
}

/// Dateiname, falls die Zeile ein Bildmarker ist.
pub fn marker_name(line: &str) -> Option<&str> {
    let name = line
        .trim()
        .strip_prefix(MARKER_START)?
        .strip_suffix(MARKER_END)?;
    // Nur einfache Dateinamen – keine Pfade.
    let plain = !name.is_empty() && !name.contains(['/', '\\', '(', ')']);
    plain.then_some(name)
}

/// Zerlegt den Text in Blöcke. Vor, zwischen und hinter Bildern steht immer ein
/// (eventuell leerer) Textblock, damit sich überall schreiben lässt.
pub fn parse(body: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut text: Vec<&str> = Vec::new();
    for line in body.split('\n') {
        match marker_name(line) {
            Some(name) => {
                blocks.push(Block::Text(text.join("\n")));
                text.clear();
                blocks.push(Block::Image(name.to_owned()));
            }
            None => text.push(line),
        }
    }
    blocks.push(Block::Text(text.join("\n")));
    blocks
}

/// Setzt Blöcke wieder zu Notiztext zusammen (Umkehrung von [`parse`]).
pub fn join(blocks: &[Block]) -> String {
    blocks
        .iter()
        .map(|block| match block {
            Block::Text(text) => text.clone(),
            Block::Image(name) => marker(name),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Dateinamen aller im Text eingebetteten Bilder, in Reihenfolge.
pub fn image_names(body: &str) -> Vec<String> {
    body.split('\n')
        .filter_map(marker_name)
        .map(str::to_owned)
        .collect()
}

/// Fügt Bilder in den Textblock `block_index` an der Zeichenposition `char_index`
/// ein. Passt die Position nicht (mehr), landen die Bilder am Ende.
pub fn insert_images(body: &str, position: Option<(usize, usize)>, names: &[String]) -> String {
    let mut blocks = parse(body);
    let images = names.iter().map(|name| Block::Image(name.clone()));

    let target = position.and_then(|(block_index, char_index)| match blocks.get(block_index) {
        Some(Block::Text(text)) => Some((block_index, char_index.min(text.chars().count()))),
        _ => None,
    });
    match target {
        Some((block_index, char_index)) => {
            let Block::Text(text) = blocks[block_index].clone() else {
                unreachable!("oben geprüft: der Block ist Text");
            };
            let split = text
                .char_indices()
                .nth(char_index)
                .map_or(text.len(), |(byte_index, _)| byte_index);
            // Am Zeilenanfang/-ende keine zusätzlichen Leerzeilen erzeugen.
            let before = text[..split].strip_suffix('\n').unwrap_or(&text[..split]);
            let after = text[split..].strip_prefix('\n').unwrap_or(&text[split..]);
            let mut replacement = vec![Block::Text(before.to_owned())];
            for (index, image) in images.enumerate() {
                if index > 0 {
                    replacement.push(Block::Text(String::new()));
                }
                replacement.push(image);
            }
            replacement.push(Block::Text(after.to_owned()));
            blocks.splice(block_index..=block_index, replacement);
        }
        None => {
            for image in images {
                blocks.push(image);
                blocks.push(Block::Text(String::new()));
            }
        }
    }
    join(&blocks)
}

/// Entfernt ein Bild aus dem Text; die Textblöcke davor und danach rücken zusammen.
pub fn remove_image(body: &str, name: &str) -> String {
    let lines: Vec<&str> = body
        .split('\n')
        .filter(|line| marker_name(line) != Some(name))
        .collect();
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Block {
        Block::Text(s.to_owned())
    }

    fn image(s: &str) -> Block {
        Block::Image(s.to_owned())
    }

    #[test]
    fn join_after_parse_is_stable() {
        for body in [
            "",
            "nur text\nzweite zeile",
            "a\n![bild](x.png)\nb",
            "![bild](x.png)",
            "![bild](x.png)\n![bild](y.png)",
            "a\n\n![bild](x.png)\n\nb\n",
        ] {
            // Das erste Zusammensetzen ergänzt höchstens leere Zeilen um Bilder herum;
            // danach ändert sich nichts mehr, und kein Bild geht verloren.
            let once = join(&parse(body));
            assert_eq!(join(&parse(&once)), once, "{body:?}");
            assert_eq!(image_names(&once), image_names(body), "{body:?}");
            assert_eq!(once.replace('\n', ""), body.replace('\n', ""), "{body:?}");
        }
        assert_eq!(join(&parse("a\n![bild](x.png)\nb")), "a\n![bild](x.png)\nb");
    }

    #[test]
    fn parse_separates_images_with_text() {
        assert_eq!(
            parse("a\n![bild](x.png)\n![bild](y.png)"),
            vec![
                text("a"),
                image("x.png"),
                text(""),
                image("y.png"),
                text("")
            ]
        );
        assert_eq!(parse("kein ![bild](x.png) mitten im Satz").len(), 1);
        assert_eq!(parse("![bild](../geheim.png)").len(), 1);
    }

    #[test]
    fn insert_at_cursor_and_at_end() {
        let names = vec!["n.png".to_owned()];
        assert_eq!(
            insert_images("hallo welt", Some((0, 5)), &names),
            "hallo\n![bild](n.png)\n welt"
        );
        assert_eq!(
            insert_images("zeile1\nzeile2", Some((0, 7)), &names),
            "zeile1\n![bild](n.png)\nzeile2"
        );
        assert_eq!(insert_images("abc", None, &names), "abc\n![bild](n.png)\n");
        assert_eq!(
            insert_images("abc", Some((9, 0)), &names),
            "abc\n![bild](n.png)\n"
        );
        // Position hinter dem Textende und mit Umlauten (Zeichen, nicht Bytes).
        assert_eq!(
            insert_images("äö", Some((0, 1)), &names),
            "ä\n![bild](n.png)\nö"
        );
        assert_eq!(
            insert_images("äö", Some((0, 99)), &names),
            "äö\n![bild](n.png)\n"
        );
        let two = vec!["a.png".to_owned(), "b.png".to_owned()];
        assert_eq!(image_names(&insert_images("x", Some((0, 1)), &two)), two);
    }

    #[test]
    fn remove_image_keeps_text() {
        let body = "a\n![bild](x.png)\nb\n![bild](y.png)";
        assert_eq!(remove_image(body, "x.png"), "a\nb\n![bild](y.png)");
        assert_eq!(image_names(body), vec!["x.png", "y.png"]);
    }
}

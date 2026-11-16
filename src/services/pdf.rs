//! Kleiner PDF-Schreiber für den Notiz-Export: Text (Standardschrift Helvetica)
//! und JPEG-Bilder auf A4-Seiten. Bewusst ohne zusätzliche Abhängigkeit.

const PAGE_WIDTH: f32 = 595.0;
const PAGE_HEIGHT: f32 = 842.0;
const MARGIN: f32 = 56.0;
const TITLE_SIZE: f32 = 18.0;
const BODY_SIZE: f32 = 11.0;
const LINE_GAP: f32 = 1.4;

/// Ein Bild für das PDF: bereits als JPEG kodiert.
pub struct PdfImage {
    pub width: u32,
    pub height: u32,
    pub jpeg: Vec<u8>,
}

/// Ein Abschnitt des Dokuments, in Lesereihenfolge.
pub enum PdfBlock {
    Text(String),
    Image(PdfImage),
}

enum Op {
    Text {
        bold: bool,
        size: f32,
        x: f32,
        y: f32,
        text: Vec<u8>,
    },
    Image {
        index: usize,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
}

/// Zeichen → Windows-1252 (die Kodierung der PDF-Standardschriften).
/// Nicht darstellbare Zeichen werden zu „?“.
fn encode_char(c: char) -> u8 {
    match c as u32 {
        0x20..=0x7E | 0xA0..=0xFF => c as u8,
        _ => match c {
            '€' => 0x80,
            '‚' => 0x82,
            '„' => 0x84,
            '…' => 0x85,
            '‘' => 0x91,
            '’' => 0x92,
            '“' => 0x93,
            '”' => 0x94,
            '•' => 0x95,
            '–' => 0x96,
            '—' => 0x97,
            '™' => 0x99,
            '\t' => b' ',
            _ => b'?',
        },
    }
}

/// Ungefähre Zeichenbreite in Helvetica (Anteil der Schriftgröße) – reicht für den Umbruch.
fn char_width(c: char) -> f32 {
    match c {
        ' ' | 'i' | 'j' | 'l' | 'I' | '.' | ',' | ':' | ';' | '!' | '|' | '\'' => 0.28,
        'f' | 't' | 'r' | '(' | ')' | '[' | ']' | '-' | '/' => 0.34,
        'm' | 'w' | 'M' | 'W' | '@' | '%' => 0.89,
        'A'..='Z' | 'Ä' | 'Ö' | 'Ü' => 0.72,
        _ => 0.56,
    }
}

fn text_width(text: &str, size: f32) -> f32 {
    text.chars().map(char_width).sum::<f32>() * size
}

/// Bricht einen Absatz an Wortgrenzen um; überlange Wörter werden hart getrennt.
fn wrap(paragraph: &str, size: f32, max_width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in paragraph.split(' ') {
        let candidate = if current.is_empty() {
            word.to_owned()
        } else {
            format!("{current} {word}")
        };
        if text_width(&candidate, size) <= max_width {
            current = candidate;
            continue;
        }
        if !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }
        // Das Wort allein ist zu lang → zeichenweise aufteilen.
        for c in word.chars() {
            if text_width(&current, size) + char_width(c) * size > max_width {
                lines.push(std::mem::take(&mut current));
            }
            current.push(c);
        }
    }
    lines.push(current);
    lines
}

fn escape(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 2);
    for c in text.chars() {
        let byte = encode_char(c);
        if matches!(byte, b'(' | b')' | b'\\') {
            out.push(b'\\');
        }
        out.push(byte);
    }
    out
}

/// Verteilt Titel, Text und Bilder in Lesereihenfolge auf Seiten.
fn layout(title: &str, blocks: &[PdfBlock]) -> Vec<Vec<Op>> {
    let content_width = PAGE_WIDTH - 2.0 * MARGIN;
    let bottom = MARGIN;
    let mut pages: Vec<Vec<Op>> = vec![Vec::new()];
    let mut y = PAGE_HEIGHT - MARGIN;

    let place_line = |pages: &mut Vec<Vec<Op>>, y: &mut f32, bold: bool, size: f32, line: &str| {
        let step = size * LINE_GAP;
        if *y - step < bottom {
            pages.push(Vec::new());
            *y = PAGE_HEIGHT - MARGIN;
        }
        *y -= step;
        if !line.is_empty() {
            pages
                .last_mut()
                .expect("mindestens eine Seite")
                .push(Op::Text {
                    bold,
                    size,
                    x: MARGIN,
                    y: *y,
                    text: escape(line),
                });
        }
    };

    if !title.trim().is_empty() {
        for line in wrap(title.trim(), TITLE_SIZE, content_width) {
            place_line(&mut pages, &mut y, true, TITLE_SIZE, &line);
        }
        place_line(&mut pages, &mut y, false, BODY_SIZE, "");
    }

    let mut image_index = 0;
    for block in blocks {
        match block {
            PdfBlock::Text(text) => {
                for paragraph in text.lines() {
                    for line in wrap(paragraph.trim_end(), BODY_SIZE, content_width) {
                        place_line(&mut pages, &mut y, false, BODY_SIZE, &line);
                    }
                }
            }
            PdfBlock::Image(image) => {
                let index = image_index;
                image_index += 1;
                if image.width == 0 || image.height == 0 {
                    continue;
                }
                let max_height = PAGE_HEIGHT - 2.0 * MARGIN;
                // Bilder nie vergrößern, höchstens auf Seitenbreite/-höhe verkleinern.
                let scale = (content_width / image.width as f32)
                    .min(max_height / image.height as f32)
                    .min(0.75);
                let (width, height) = (image.width as f32 * scale, image.height as f32 * scale);
                y -= 10.0;
                if y - height < bottom {
                    pages.push(Vec::new());
                    y = PAGE_HEIGHT - MARGIN;
                }
                y -= height;
                pages
                    .last_mut()
                    .expect("mindestens eine Seite")
                    .push(Op::Image {
                        index,
                        x: MARGIN,
                        y,
                        width,
                        height,
                    });
                y -= 6.0;
            }
        }
    }
    pages
}

/// Erzeugt ein vollständiges PDF-Dokument.
pub fn build(title: &str, blocks: &[PdfBlock]) -> Vec<u8> {
    let pages = layout(title, blocks);
    let images: Vec<&PdfImage> = blocks
        .iter()
        .filter_map(|block| match block {
            PdfBlock::Image(image) => Some(image),
            PdfBlock::Text(_) => None,
        })
        .collect();

    // Objektnummern: 1 Katalog, 2 Seitenbaum, 3/4 Schriften, dann Bilder, dann je Seite 2 Objekte.
    let first_image = 5;
    let first_page = first_image + images.len();
    let mut objects: Vec<Vec<u8>> = Vec::new();

    objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    let kids: Vec<String> = (0..pages.len())
        .map(|i| format!("{} 0 R", first_page + 2 * i))
        .collect();
    objects.push(
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            pages.len()
        )
        .into_bytes(),
    );
    for font in ["Helvetica", "Helvetica-Bold"] {
        objects.push(
            format!(
                "<< /Type /Font /Subtype /Type1 /BaseFont /{font} /Encoding /WinAnsiEncoding >>"
            )
            .into_bytes(),
        );
    }
    for image in &images {
        let mut object = format!(
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB \
             /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
            image.width,
            image.height,
            image.jpeg.len()
        )
        .into_bytes();
        object.extend_from_slice(&image.jpeg);
        object.extend_from_slice(b"\nendstream");
        objects.push(object);
    }

    let xobjects: Vec<String> = (0..images.len())
        .map(|i| format!("/Im{i} {} 0 R", first_image + i))
        .collect();
    let resources = format!(
        "<< /Font << /F1 3 0 R /F2 4 0 R >> /XObject << {} >> >>",
        xobjects.join(" ")
    );
    for (page_index, ops) in pages.iter().enumerate() {
        let content_id = first_page + 2 * page_index + 1;
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_WIDTH} {PAGE_HEIGHT}] \
                 /Resources {resources} /Contents {content_id} 0 R >>"
            )
            .into_bytes(),
        );

        let mut stream: Vec<u8> = Vec::new();
        for op in ops {
            match op {
                Op::Text {
                    bold,
                    size,
                    x,
                    y,
                    text,
                } => {
                    let font = if *bold { "F2" } else { "F1" };
                    stream.extend_from_slice(
                        format!("BT /{font} {size} Tf {x:.2} {y:.2} Td (").as_bytes(),
                    );
                    stream.extend_from_slice(text);
                    stream.extend_from_slice(b") Tj ET\n");
                }
                Op::Image {
                    index,
                    x,
                    y,
                    width,
                    height,
                } => {
                    stream.extend_from_slice(
                        format!("q {width:.2} 0 0 {height:.2} {x:.2} {y:.2} cm /Im{index} Do Q\n")
                            .as_bytes(),
                    );
                }
            }
        }
        let mut object = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
        object.extend_from_slice(&stream);
        object.extend_from_slice(b"\nendstream");
        objects.push(object);
    }

    // Datei zusammensetzen: Kopf, Objekte, Querverweistabelle, Anhang.
    let mut pdf: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        pdf.extend_from_slice(object);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref_start = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_start}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_long_lines_and_words() {
        let lines = wrap(&"wort ".repeat(60), BODY_SIZE, 200.0);
        assert!(lines.len() > 3);
        assert!(lines.iter().all(|l| text_width(l, BODY_SIZE) <= 200.0));
        let long = wrap(&"x".repeat(300), BODY_SIZE, 200.0);
        assert!(long.len() > 1);
    }

    #[test]
    fn encodes_german_text() {
        assert_eq!(encode_char('ä'), 0xE4);
        assert_eq!(encode_char('€'), 0x80);
        assert_eq!(encode_char('„'), 0x84);
        assert_eq!(encode_char('😀'), b'?');
        assert_eq!(escape("a(b)"), b"a\\(b\\)".to_vec());
    }

    #[test]
    fn many_lines_span_pages() {
        let blocks = [PdfBlock::Text("Zeile\n".repeat(200))];
        let pages = layout("Titel", &blocks);
        assert!(pages.len() >= 3);
        let pdf = build("Titel", &blocks);
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(pdf.ends_with(b"%%EOF\n"));
    }
}

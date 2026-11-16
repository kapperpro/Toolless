//! Minimaler Leser für Minecrafts NBT-Format (unkomprimiert, Big-Endian) –
//! gerade genug, um Angaben aus `level.dat` zu holen.

/// Schachtelungstiefe, ab der abgebrochen wird (Schutz vor defekten Dateien).
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub enum Nbt {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    String(String),
    List(Vec<Nbt>),
    Compound(Vec<(String, Nbt)>),
    /// Zahlenfelder (Byte-/Int-/Long-Arrays) – ihr Inhalt wird hier nicht gebraucht.
    Array,
}

impl Nbt {
    /// Kind eines Compounds mit diesem Namen.
    pub fn get(&self, name: &str) -> Option<&Nbt> {
        match self {
            Nbt::Compound(children) => children
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// Folgt einem Pfad wie `["Data", "Version", "Name"]`.
    pub fn path(&self, names: &[&str]) -> Option<&Nbt> {
        names.iter().try_fold(self, |node, name| node.get(name))
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Nbt::Byte(v) => Some(*v as i64),
            Nbt::Short(v) => Some(*v as i64),
            Nbt::Int(v) => Some(*v as i64),
            Nbt::Long(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Nbt::String(text) => Some(text),
            _ => None,
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take(&mut self, count: usize) -> Option<&[u8]> {
        let end = self.pos.checked_add(count)?;
        let slice = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_be_bytes(self.take(2)?.try_into().ok()?))
    }

    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }

    fn i64(&mut self) -> Option<i64> {
        Some(i64::from_be_bytes(self.take(8)?.try_into().ok()?))
    }

    fn string(&mut self) -> Option<String> {
        let len = self.u16()? as usize;
        Some(String::from_utf8_lossy(self.take(len)?).into_owned())
    }

    /// Überspringt ein Zahlenfeld mit `width` Bytes je Element.
    fn skip_array(&mut self, width: usize) -> Option<Nbt> {
        let len = usize::try_from(self.i32()?).ok()?;
        self.take(len.checked_mul(width)?)?;
        Some(Nbt::Array)
    }

    fn payload(&mut self, tag: u8, depth: usize) -> Option<Nbt> {
        if depth > MAX_DEPTH {
            return None;
        }
        Some(match tag {
            1 => Nbt::Byte(self.u8()? as i8),
            2 => Nbt::Short(self.u16()? as i16),
            3 => Nbt::Int(self.i32()?),
            4 => Nbt::Long(self.i64()?),
            5 => Nbt::Float(f32::from_bits(self.i32()? as u32)),
            6 => Nbt::Double(f64::from_bits(self.i64()? as u64)),
            7 => self.skip_array(1)?,
            8 => Nbt::String(self.string()?),
            9 => {
                let element = self.u8()?;
                let len = usize::try_from(self.i32()?).unwrap_or(0);
                // Jedes Element braucht mindestens ein Byte – so sprengt eine falsche Länge nichts.
                if element != 0 && len > self.bytes.len() - self.pos {
                    return None;
                }
                let mut items = Vec::new();
                for _ in 0..len {
                    items.push(self.payload(element, depth + 1)?);
                }
                Nbt::List(items)
            }
            10 => {
                let mut children = Vec::new();
                loop {
                    let child = self.u8()?;
                    if child == 0 {
                        break;
                    }
                    let name = self.string()?;
                    children.push((name, self.payload(child, depth + 1)?));
                }
                Nbt::Compound(children)
            }
            11 => self.skip_array(4)?,
            12 => self.skip_array(8)?,
            _ => return None,
        })
    }
}

/// Liest ein unkomprimiertes NBT-Dokument (Wurzel ist ein benannter Compound).
pub fn parse(bytes: &[u8]) -> Option<Nbt> {
    let mut reader = Reader { bytes, pos: 0 };
    if reader.u8()? != 10 {
        return None;
    }
    reader.string()?; // Name der Wurzel, meist leer
    reader.payload(10, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(tag: u8, name: &str) -> Vec<u8> {
        let mut out = vec![tag];
        out.extend((name.len() as u16).to_be_bytes());
        out.extend(name.as_bytes());
        out
    }

    /// Baut ein kleines `level.dat`-ähnliches Dokument von Hand.
    fn sample() -> Vec<u8> {
        let mut b = named(10, "");
        b.extend(named(10, "Data"));
        b.extend(named(8, "LevelName"));
        b.extend(9u16.to_be_bytes());
        b.extend("Neue Welt".as_bytes()); // 9 Bytes
        b.extend(named(4, "RandomSeed"));
        b.extend((-42i64).to_be_bytes());
        b.extend(named(11, "Zahlen"));
        b.extend(2i32.to_be_bytes());
        b.extend([0u8; 8]);
        b.extend(named(9, "Liste"));
        b.push(3);
        b.extend(2i32.to_be_bytes());
        b.extend(7i32.to_be_bytes());
        b.extend(8i32.to_be_bytes());
        b.extend(named(10, "Version"));
        b.extend(named(8, "Name"));
        b.extend(6u16.to_be_bytes());
        b.extend("1.21.4".as_bytes());
        b.push(0); // Ende Version
        b.push(0); // Ende Data
        b.push(0); // Ende Wurzel
        b
    }

    #[test]
    fn reads_nested_values() {
        let root = parse(&sample()).expect("gültiges Dokument");
        assert_eq!(
            root.path(&["Data", "LevelName"]).and_then(Nbt::as_str),
            Some("Neue Welt")
        );
        assert_eq!(
            root.path(&["Data", "RandomSeed"]).and_then(Nbt::as_i64),
            Some(-42)
        );
        assert_eq!(
            root.path(&["Data", "Version", "Name"])
                .and_then(Nbt::as_str),
            Some("1.21.4")
        );
        assert_eq!(
            root.path(&["Data", "Liste"]),
            Some(&Nbt::List(vec![Nbt::Int(7), Nbt::Int(8)]))
        );
        assert_eq!(root.path(&["Data", "Zahlen"]), Some(&Nbt::Array));
        assert_eq!(root.path(&["Data", "Fehlt"]), None);
    }

    #[test]
    fn rejects_truncated_and_wrong_input() {
        let bytes = sample();
        assert!(parse(&bytes[..bytes.len() - 5]).is_none());
        assert!(parse(&[]).is_none());
        assert!(parse(&[8, 0, 0]).is_none());
        // Liste mit absurd großer Länge darf nicht zu riesigen Reservierungen führen.
        let mut bad = named(10, "");
        bad.extend(named(9, "L"));
        bad.push(1);
        bad.extend(i32::MAX.to_be_bytes());
        assert!(parse(&bad).is_none());
    }
}

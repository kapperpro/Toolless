//! Rechenlogik des Taschenrechners: kleiner rekursiver Parser.
//! Unterstützt + − × ÷, Klammern, Vorzeichen und Prozent (Postfix).

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn expression(&mut self) -> Result<f64, String> {
        let mut value = self.term()?;
        while let Some(op @ ('+' | '-')) = self.peek() {
            self.pos += 1;
            let right = self.term()?;
            value = if op == '+' {
                value + right
            } else {
                value - right
            };
        }
        Ok(value)
    }

    fn term(&mut self) -> Result<f64, String> {
        let mut value = self.unary()?;
        while let Some(op @ ('*' | '/')) = self.peek() {
            self.pos += 1;
            let right = self.unary()?;
            if op == '/' && right == 0.0 {
                return Err("Division durch 0".into());
            }
            value = if op == '*' {
                value * right
            } else {
                value / right
            };
        }
        Ok(value)
    }

    fn unary(&mut self) -> Result<f64, String> {
        match self.peek() {
            Some('-') => {
                self.pos += 1;
                Ok(-self.unary()?)
            }
            Some('+') => {
                self.pos += 1;
                self.unary()
            }
            _ => self.postfix(),
        }
    }

    fn postfix(&mut self) -> Result<f64, String> {
        let mut value = self.primary()?;
        while self.peek() == Some('%') {
            self.pos += 1;
            value /= 100.0;
        }
        Ok(value)
    }

    fn primary(&mut self) -> Result<f64, String> {
        if self.peek() == Some('(') {
            self.pos += 1;
            let value = self.expression()?;
            if self.peek() != Some(')') {
                return Err("Klammer fehlt".into());
            }
            self.pos += 1;
            return Ok(value);
        }
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == '.') {
            self.pos += 1;
        }
        let literal: String = self.chars[start..self.pos].iter().collect();
        if literal.is_empty() {
            return Err("Syntaxfehler".into());
        }
        // „5.“ ist erlaubt, „1.2.3“ nicht.
        literal
            .trim_end_matches('.')
            .parse::<f64>()
            .map_err(|_| "Syntaxfehler".to_string())
    }
}

/// Wertet einen Ausdruck aus. Akzeptiert die Anzeige-Zeichen × ÷ − und das Dezimalkomma.
pub fn evaluate(input: &str) -> Result<f64, String> {
    let chars: Vec<char> = input
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| match c {
            '×' => '*',
            '÷' => '/',
            '−' => '-',
            ',' => '.',
            other => other,
        })
        .collect();
    if chars.is_empty() {
        return Ok(0.0);
    }
    let mut parser = Parser { chars, pos: 0 };
    let value = parser.expression()?;
    if parser.pos < parser.chars.len() {
        return Err("Syntaxfehler".into());
    }
    if !value.is_finite() {
        return Err("Ergebnis zu groß".into());
    }
    Ok(value)
}

/// Rundet Gleitkomma-Artefakte weg (0,1 + 0,2 → 0,3) und nutzt das Dezimalkomma.
pub fn format_number(value: f64) -> String {
    let magnitude = value.abs();
    let text = if magnitude != 0.0 && !(1e-9..1e15).contains(&magnitude) {
        format!("{value:e}")
    } else {
        let fixed = format!("{value:.10}");
        let trimmed = fixed.trim_end_matches('0').trim_end_matches('.');
        if trimmed == "-0" {
            "0".to_owned()
        } else {
            trimmed.to_owned()
        }
    };
    text.replace('.', ",")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calc(expr: &str) -> String {
        format_number(evaluate(expr).unwrap())
    }

    #[test]
    fn operator_precedence_and_parentheses() {
        assert_eq!(calc("1+2×3"), "7");
        assert_eq!(calc("(1+2)×3"), "9");
        assert_eq!(calc("8÷4÷2"), "1");
        assert_eq!(calc("2×-3"), "-6");
        assert_eq!(calc("−(3−5)"), "2");
    }

    #[test]
    fn decimals_and_percent() {
        assert_eq!(calc("0,1+0,2"), "0,3");
        assert_eq!(calc("50%"), "0,5");
        assert_eq!(calc("200×10%"), "20");
        assert_eq!(calc("5,"), "5");
        assert_eq!(calc("12×3+0,5"), "36,5");
        assert_eq!(calc("1÷3"), "0,3333333333");
    }

    #[test]
    fn errors() {
        assert_eq!(evaluate("1÷0").unwrap_err(), "Division durch 0");
        assert!(evaluate("2+").is_err());
        assert!(evaluate("(1").is_err());
        assert!(evaluate("1,2,3").is_err());
        assert!(evaluate("12(").is_err());
    }
}

// Rechenlogik des Taschenrechners: kleiner rekursiver Parser statt `eval`.
// Unterstützt + − × ÷, Klammern, Vorzeichen und Prozent (Postfix).

export function evaluate(input: string): number {
  const s = input.replace(/\s+/g, "").replace(/×/g, "*").replace(/÷/g, "/").replace(/−/g, "-").replace(/,/g, ".");
  let i = 0;

  const expression = (): number => {
    let value = term();
    while (s[i] === "+" || s[i] === "-") {
      const op = s[i++];
      const right = term();
      value = op === "+" ? value + right : value - right;
    }
    return value;
  };

  const term = (): number => {
    let value = unary();
    while (s[i] === "*" || s[i] === "/") {
      const op = s[i++];
      const right = unary();
      if (op === "/" && right === 0) throw new Error("Division durch 0");
      value = op === "*" ? value * right : value / right;
    }
    return value;
  };

  const unary = (): number => {
    if (s[i] === "-") {
      i++;
      return -unary();
    }
    if (s[i] === "+") {
      i++;
      return unary();
    }
    return postfix();
  };

  const postfix = (): number => {
    let value = primary();
    while (s[i] === "%") {
      i++;
      value /= 100;
    }
    return value;
  };

  const primary = (): number => {
    if (s[i] === "(") {
      i++;
      const value = expression();
      if (s[i] !== ")") throw new Error("Klammer fehlt");
      i++;
      return value;
    }
    const match = /^(\d+\.?\d*|\.\d+)/.exec(s.slice(i));
    if (!match) throw new Error("Syntaxfehler");
    i += match[0].length;
    return Number(match[0]);
  };

  if (s.length === 0) return 0;
  const result = expression();
  if (i < s.length) throw new Error("Syntaxfehler");
  if (!Number.isFinite(result)) throw new Error("Ergebnis zu groß");
  return result;
}

/** Rundet Gleitkomma-Artefakte weg (0.1 + 0.2 → 0.3) und nutzt das Dezimalkomma. */
export function formatNumber(value: number): string {
  return String(Number(value.toPrecision(12))).replace(".", ",");
}

import { useCallback, useEffect, useState } from "react";
import { evaluate, formatNumber } from "../lib/calc";

interface HistoryEntry {
  expression: string;
  result: string;
}

type Kind = "fn" | "op" | "num";

const KEYS: { label: string; kind: Kind; wide?: boolean }[] = [
  { label: "AC", kind: "fn" },
  { label: "(", kind: "fn" },
  { label: ")", kind: "fn" },
  { label: "÷", kind: "op" },
  { label: "7", kind: "num" },
  { label: "8", kind: "num" },
  { label: "9", kind: "num" },
  { label: "×", kind: "op" },
  { label: "4", kind: "num" },
  { label: "5", kind: "num" },
  { label: "6", kind: "num" },
  { label: "−", kind: "op" },
  { label: "1", kind: "num" },
  { label: "2", kind: "num" },
  { label: "3", kind: "num" },
  { label: "+", kind: "op" },
  { label: "±", kind: "num" },
  { label: "0", kind: "num" },
  { label: ",", kind: "num" },
  { label: "=", kind: "op" },
  { label: "%", kind: "fn", wide: true },
  { label: "⌫", kind: "fn", wide: true },
];

const OPERATORS = "+−×÷";

export default function Calculator() {
  const [expression, setExpression] = useState("");
  const [error, setError] = useState<string | null>(null);
  /** Nach „=“ ersetzt die nächste Zifferneingabe das Ergebnis. */
  const [justEvaluated, setJustEvaluated] = useState(false);
  const [history, setHistory] = useState<HistoryEntry[]>([]);

  const press = useCallback(
    (key: string) => {
      setError(null);
      switch (key) {
        case "AC":
          setExpression("");
          setJustEvaluated(false);
          return;
        case "⌫":
          setExpression((e) => (justEvaluated ? "" : e.slice(0, -1)));
          setJustEvaluated(false);
          return;
        case "=": {
          if (!expression) return;
          try {
            const result = formatNumber(evaluate(expression));
            setHistory((h) => [{ expression, result }, ...h].slice(0, 30));
            setExpression(result);
            setJustEvaluated(true);
          } catch (err) {
            setError(err instanceof Error ? err.message : "Fehler");
          }
          return;
        }
        case "±":
          setExpression((e) => {
            if (!e) return "−";
            if (/^−\(.*\)$/.test(e)) return e.slice(2, -1);
            if (/^−[\d,]+$/.test(e)) return e.slice(1);
            return /^[\d,]+$/.test(e) ? `−${e}` : `−(${e})`;
          });
          setJustEvaluated(false);
          return;
        default: {
          const isOperator = OPERATORS.includes(key);
          setExpression((e) => {
            // Neues Rechnen nach „=“: Ziffern ersetzen, Operatoren rechnen weiter.
            if (justEvaluated && !isOperator && key !== "%") return key;
            // Zwei Operatoren hintereinander: der letzte gewinnt.
            if (isOperator && e && OPERATORS.includes(e[e.length - 1]) && key !== "−") {
              return e.slice(0, -1) + key;
            }
            return e + key;
          });
          setJustEvaluated(false);
        }
      }
    },
    [expression, justEvaluated],
  );

  // Tastatureingabe wie bei einem nativen Rechner.
  useEffect(() => {
    const map: Record<string, string> = {
      "*": "×",
      "/": "÷",
      "-": "−",
      ".": ",",
      Enter: "=",
      Backspace: "⌫",
      Escape: "AC",
      Delete: "AC",
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      const key = map[event.key] ?? event.key;
      if (/^[0-9+,()%=]$/.test(key) || ["×", "÷", "−", "⌫", "AC"].includes(key)) {
        event.preventDefault();
        press(key);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [press]);

  let preview = "";
  if (expression && !justEvaluated) {
    try {
      preview = formatNumber(evaluate(expression));
    } catch {
      preview = "";
    }
  }

  return (
    <div className="calc-layout">
      <div className="calc">
        <div className="calc-display" aria-live="polite">
          <div className="calc-preview">{error ?? (preview ? `= ${preview}` : " ")}</div>
          <div className="calc-expression">{expression || "0"}</div>
        </div>
        <div className="calc-keys">
          {KEYS.map((key) => (
            <button
              key={key.label}
              className={`calc-key ${key.kind}${key.wide ? " wide" : ""}`}
              onClick={() => press(key.label)}
            >
              {key.label}
            </button>
          ))}
        </div>
      </div>
      <div className="card calc-history">
        <div className="card-title">Verlauf</div>
        {history.length === 0 && <p className="muted">Noch keine Rechnungen.</p>}
        <ul className="plain-list">
          {history.map((entry, index) => (
            <li key={index}>
              <button
                className="history-entry"
                title="Ergebnis übernehmen"
                onClick={() => {
                  setExpression(entry.result);
                  setJustEvaluated(true);
                }}
              >
                <span className="muted">{entry.expression}</span>
                <strong>= {entry.result}</strong>
              </button>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}

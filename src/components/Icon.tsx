// Schlanke Linien-Icons (24er Raster) im Stil von SF Symbols.
const PATHS = {
  music: "M9 18V5l11-2v13M9 18a3 3 0 1 1-6 0 3 3 0 0 1 6 0Zm11-2a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z",
  clipboard:
    "M9 4h6a1 1 0 0 1 1 1v1H8V5a1 1 0 0 1 1-1ZM8 5H6.5A1.5 1.5 0 0 0 5 6.5v12A1.5 1.5 0 0 0 6.5 20h11a1.5 1.5 0 0 0 1.5-1.5v-12A1.5 1.5 0 0 0 17.5 5H16M9 11h6M9 15h4",
  calculator:
    "M7 3h10a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2Zm1.5 4h7v3h-7V7Zm0 7h.01M12 14h.01M15.5 14h.01M8.5 17.5h.01M12 17.5h.01M15.5 17.5h.01",
  note: "M6 3h9l4 4v12a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2Zm9 0v4h4M8 12h8M8 16h5",
  grid: "M4 4h6v6H4V4Zm10 0h6v6h-6V4ZM4 14h6v6H4v-6Zm10 0h6v6h-6v-6Z",
  globe:
    "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18ZM3 12h18M12 3c2.5 2.6 3.8 5.6 3.8 9s-1.3 6.4-3.8 9c-2.5-2.6-3.8-5.6-3.8-9S9.5 5.6 12 3Z",
  gear: "M12 15.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7Zm0-12.5v2.5m0 13v2.5M3 12h2.5m13 0H21M5.6 5.6l1.8 1.8m9.2 9.2 1.8 1.8m0-12.8-1.8 1.8m-9.2 9.2-1.8 1.8",
  play: "M8 5.5v13l11-6.5L8 5.5Z",
  pause: "M8 5h3v14H8V5Zm5 0h3v14h-3V5Z",
  next: "M5 5.5v13l9-6.5-9-6.5ZM17 5h2v14h-2V5Z",
  previous: "M19 5.5v13l-9-6.5 9-6.5ZM5 5h2v14H5V5Z",
  copy: "M9 9h10v11H9V9Zm-4 6V4h10",
  trash: "M5 7h14M10 7V4h4v3m-7 0 1 13h8l1-13M10 11v6m4-6v6",
  pencil: "m4 20 1-4L16 5l3 3L8 19l-4 1Zm10-13 3 3",
  plus: "M12 5v14M5 12h14",
  refresh: "M20 12a8 8 0 1 1-2.3-5.6M20 4v4.5h-4.5",
  external: "M14 4h6v6m0-6-9 9M10 6H5v13h13v-5",
  check: "m5 12.5 4.5 4.5L19 7.5",
  search: "M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14Zm5-2 4 4",
} as const;

export type IconName = keyof typeof PATHS;

const FILLED: ReadonlySet<IconName> = new Set<IconName>(["play", "pause", "next", "previous"]);

export function Icon({ name, size = 18 }: { name: IconName; size?: number }) {
  const filled = FILLED.has(name);
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill={filled ? "currentColor" : "none"}
      stroke={filled ? "none" : "currentColor"}
      strokeWidth={1.7}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={PATHS[name]} />
    </svg>
  );
}

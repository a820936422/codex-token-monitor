const COLUMN_WIDTHS_KEY = "wtm.columnWidths";
export const MAX_COLUMN_WIDTH = 720;
export const COLUMN_CONFIG = {
  time: { defaultWidth: 145, minWidth: 110 },
  project: { defaultWidth: 170, minWidth: 110 },
  conversation: { defaultWidth: 320, minWidth: 190 },
  model: { defaultWidth: 180, minWidth: 130 },
  input: { defaultWidth: 100, minWidth: 80 },
  cached: { defaultWidth: 100, minWidth: 80 },
  output: { defaultWidth: 100, minWidth: 80 },
  cache: { defaultWidth: 130, minWidth: 120 },
} as const;

export type ColumnKey = keyof typeof COLUMN_CONFIG;
export type ColumnWidths = Record<ColumnKey, number>;
export const columnKeys = Object.keys(COLUMN_CONFIG) as ColumnKey[];

export function saved(key: string) {
  try {
    return localStorage.getItem(key) || "";
  } catch {
    return "";
  }
}

export function persist(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {}
}

function defaultColumnWidths(): ColumnWidths {
  return Object.fromEntries(
    columnKeys.map((key) => [key, COLUMN_CONFIG[key].defaultWidth]),
  ) as ColumnWidths;
}

export function clampColumnWidth(key: ColumnKey, width: number) {
  return Math.max(COLUMN_CONFIG[key].minWidth, Math.min(MAX_COLUMN_WIDTH, Math.round(width)));
}

export function loadColumnWidths(): ColumnWidths {
  const defaults = defaultColumnWidths();
  const raw = saved(COLUMN_WIDTHS_KEY);
  if (!raw) return defaults;
  try {
    const parsed = JSON.parse(raw) as Partial<Record<ColumnKey, unknown>>;
    for (const key of columnKeys) {
      if (typeof parsed[key] === "number" && Number.isFinite(parsed[key])) {
        defaults[key] = clampColumnWidth(key, parsed[key]);
      }
    }
  } catch {}
  return defaults;
}

export function persistColumnWidths(widths: ColumnWidths) {
  persist(COLUMN_WIDTHS_KEY, JSON.stringify(widths));
}

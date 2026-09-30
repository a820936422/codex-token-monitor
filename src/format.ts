const timeFormat = new Intl.DateTimeFormat(undefined, {
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
  hour12: false,
});
export function formatEffort(value: string) {
  const normalized = value.trim().toLowerCase();
  if (normalized === "xhigh" || normalized === "extra_high" || normalized === "extra-high")
    return "Extra High";
  return (
    normalized
      .split(/[_-]+/)
      .filter(Boolean)
      .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
      .join(" ") || value
  );
}

export function formatTime(timestamp: string) {
  const date = new Date(timestamp);
  return Number.isNaN(date.getTime()) ? timestamp || "—" : timeFormat.format(date);
}

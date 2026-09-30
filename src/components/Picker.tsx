import { useEffect, useId, useMemo, useRef, useState } from "react";

export interface PickerOption {
  id: string;
  primary: string;
  secondary?: string | null;
}

interface PickerProps {
  label: string;
  selectedId: string;
  allLabel: string;
  allSecondary?: string;
  options: PickerOption[];
  wide?: boolean;
  onSelect: (id: string) => void;
}

export function Picker({
  label,
  selectedId,
  allLabel,
  allSecondary,
  options,
  wide,
  onSelect,
}: PickerProps) {
  const ref = useRef<HTMLDetailsElement>(null);
  const labelId = useId();
  const [open, setOpen] = useState(false),
    [query, setQuery] = useState("");
  const selected = useMemo(
    () => options.find((option) => option.id === selectedId),
    [options, selectedId],
  );
  const matches = useMemo(
    () =>
      open
        ? options.filter((option) =>
            `${option.primary} ${option.secondary ?? ""} ${option.id}`
              .toLowerCase()
              .includes(query.toLowerCase()),
          )
        : [],
    [open, options, query],
  );
  useEffect(() => {
    if (!open) return;
    const close = (event: PointerEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) ref.current.open = false;
    };
    document.addEventListener("pointerdown", close);
    return () => document.removeEventListener("pointerdown", close);
  }, [open]);
  const choose = (id: string) => {
    onSelect(id);
    if (ref.current) ref.current.open = false;
  };

  return (
    <div className={`field ${label.toLowerCase()}-field`}>
      <span id={labelId}>{label}</span>
      <details
        ref={ref}
        className="picker"
        onToggle={(event) => {
          setOpen(event.currentTarget.open);
          if (!event.currentTarget.open) setQuery("");
        }}
        onKeyDown={(event) => {
          if (event.key === "Escape" && ref.current) {
            ref.current.open = false;
            ref.current.querySelector("summary")?.focus();
          }
        }}
      >
        <summary aria-labelledby={labelId}>
          <span className="picker-selected">
            <strong>{selected?.primary ?? allLabel}</strong>
            <small>{selected?.secondary ?? allSecondary ?? "全部"}</small>
          </span>
        </summary>
        {open && (
          <div className={`picker-menu${wide ? " wide" : ""}`}>
            <input
              className="picker-search"
              aria-label={`搜索 ${label}`}
              placeholder="搜索名称或 ID…"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
            <button
              className={`picker-option${selectedId ? "" : " selected"}`}
              type="button"
              onClick={() => choose("")}
            >
              <strong>{allLabel}</strong>
              <small>{allSecondary ?? "不限制"}</small>
            </button>
            {matches.slice(0, 200).map((option) => (
              <button
                className={`picker-option${selectedId === option.id ? " selected" : ""}`}
                type="button"
                key={option.id}
                onClick={() => choose(option.id)}
              >
                <strong>{option.primary}</strong>
                <small>{option.secondary || option.id}</small>
              </button>
            ))}
            {matches.length > 200 && (
              <p className="picker-limit">显示前 200 / {matches.length} 项，请搜索缩小范围。</p>
            )}
          </div>
        )}
      </details>
    </div>
  );
}

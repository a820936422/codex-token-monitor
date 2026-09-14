import { useRef } from "react";

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

export function Picker({ label, selectedId, allLabel, allSecondary, options, wide, onSelect }: PickerProps) {
  const ref = useRef<HTMLDetailsElement>(null);
  const selected = options.find((option) => option.id === selectedId);
  const choose = (id: string) => {
    onSelect(id);
    if (ref.current) ref.current.open = false;
  };

  return (
    <label className={`field ${label.toLowerCase()}-field`}>
      <span>{label}</span>
      <details ref={ref} className="picker">
        <summary>
          <span className="picker-selected">
            <strong>{selected?.primary ?? allLabel}</strong>
            <small>{selected?.secondary ?? allSecondary ?? "全部"}</small>
          </span>
        </summary>
        <div className={`picker-menu${wide ? " wide" : ""}`}>
          <button className={`picker-option${selectedId ? "" : " selected"}`} type="button" onClick={() => choose("")}>
            <strong>{allLabel}</strong>
            <small>{allSecondary ?? "不限制"}</small>
          </button>
          {options.map((option) => (
            <button className={`picker-option${selectedId === option.id ? " selected" : ""}`} type="button" key={option.id} onClick={() => choose(option.id)}>
              <strong>{option.primary}</strong>
              <small>{option.secondary || option.id}</small>
            </button>
          ))}
        </div>
      </details>
    </label>
  );
}

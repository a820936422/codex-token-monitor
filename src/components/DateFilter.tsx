export function DateFilter({
  from,
  to,
  onFrom,
  onTo,
  onClear,
}: {
  from: string;
  to: string;
  onFrom: (value: string) => void;
  onTo: (value: string) => void;
  onClear: () => void;
}) {
  const active = Boolean(from || to);
  return (
    <details className={`date-filter${active ? " active" : ""}`}>
      <summary title="筛选日期范围">
        <span>自定义日期</span>
        <span className="date-filter-arrow">▾</span>
      </summary>
      <div className="date-filter-menu" onClick={(event) => event.stopPropagation()}>
        <label>
          <span>From</span>
          <input type="date" value={from} onChange={(event) => onFrom(event.target.value)} />
        </label>
        <label>
          <span>To</span>
          <input type="date" value={to} onChange={(event) => onTo(event.target.value)} />
        </label>
        <button type="button" onClick={onClear} disabled={!active}>
          清除日期
        </button>
      </div>
    </details>
  );
}

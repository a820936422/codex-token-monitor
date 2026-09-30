import { COLUMN_CONFIG, MAX_COLUMN_WIDTH, clampColumnWidth, type ColumnKey } from "../columns";
export function ColumnHeader({
  column,
  width,
  numeric = false,
  children,
  onResize,
  onReset,
}: {
  column: ColumnKey;
  width: number;
  numeric?: boolean;
  children: React.ReactNode;
  onResize: (key: ColumnKey, width: number, save: boolean) => void;
  onReset: (key: ColumnKey) => void;
}) {
  const config = COLUMN_CONFIG[column];
  const beginResize = (event: React.PointerEvent<HTMLSpanElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    const target = event.currentTarget;
    const pointerId = event.pointerId;
    const startX = event.clientX;
    const startWidth = width;
    let latestWidth = width;
    document.body.classList.add("column-resizing");
    target.setPointerCapture(pointerId);
    const move = (moveEvent: PointerEvent) => {
      latestWidth = clampColumnWidth(column, startWidth + moveEvent.clientX - startX);
      onResize(column, latestWidth, false);
    };
    const finish = () => {
      if (target.hasPointerCapture(pointerId)) target.releasePointerCapture(pointerId);
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", finish);
      target.removeEventListener("pointercancel", finish);
      document.body.classList.remove("column-resizing");
      onResize(column, latestWidth, true);
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", finish);
    target.addEventListener("pointercancel", finish);
  };
  const changeWithKeyboard = (event: React.KeyboardEvent<HTMLSpanElement>) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const step = event.shiftKey ? 25 : 10;
    onResize(column, width + (event.key === "ArrowRight" ? step : -step), true);
  };
  return (
    <th className={`resizable-header${numeric ? " numeric" : ""}`}>
      {children}
      <span
        className="column-resizer"
        role="separator"
        aria-label={`调整 ${column} 列宽`}
        aria-orientation="vertical"
        aria-valuemin={config.minWidth}
        aria-valuemax={MAX_COLUMN_WIDTH}
        aria-valuenow={width}
        tabIndex={0}
        title="拖动调整列宽；双击恢复默认宽度"
        onPointerDown={beginResize}
        onDoubleClick={() => onReset(column)}
        onKeyDown={changeWithKeyboard}
      />
    </th>
  );
}

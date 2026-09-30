import { useLayoutEffect, useState, type RefObject } from "react";

export const ROW_HEIGHT = 54;
const OVERSCAN = 8;
export function useVirtualWindow(count: number, ref: RefObject<HTMLDivElement | null>) {
  const [viewport, setViewport] = useState({ top: 0, height: 500 });
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    let frame = 0;
    const measure = () => {
      frame = 0;
      const top = element.scrollTop,
        height = element.clientHeight;
      setViewport((old) => (old.top === top && old.height === height ? old : { top, height }));
    };
    const schedule = () => {
      if (!frame) frame = requestAnimationFrame(measure);
    };
    const observer = new ResizeObserver(schedule);
    observer.observe(element);
    element.addEventListener("scroll", schedule, { passive: true });
    measure();
    return () => {
      observer.disconnect();
      element.removeEventListener("scroll", schedule);
      if (frame) cancelAnimationFrame(frame);
    };
  }, [ref]);
  const visible = Math.ceil(viewport.height / ROW_HEIGHT) + OVERSCAN * 2;
  const start = Math.max(
    0,
    Math.min(Math.floor(Math.max(0, viewport.top - 44) / ROW_HEIGHT) - OVERSCAN, count - visible),
  );
  const end = Math.min(count, start + visible);
  return { start, end, before: start * ROW_HEIGHT, after: Math.max(0, count - end) * ROW_HEIGHT };
}

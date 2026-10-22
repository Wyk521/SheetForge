/** Clamp after resize/replacement as well as during native scroll. */
export function visibleRange(count: number, scroll: number, height: number, rowHeight: number): [number, number] {
  const visible = Math.max(1, Math.ceil(height / rowHeight));
  const first = Math.min(Math.max(0, count - visible), Math.max(0, Math.floor(scroll / rowHeight)));
  return [Math.max(0, first - 2), Math.min(count, first + visible + 2)];
}

/**
 * Squarified treemap layout (Bruls, Huizing and van Wijk, 2000).
 *
 * Hand-rolled rather than pulled from d3-hierarchy: module spec 04 allows
 * either and says no heavy chart library, and this is forty lines of arithmetic
 * with no dependencies and no bundle cost. Kept in its own module, with no
 * reference to a canvas or to Svelte, so the geometry can be tested as
 * arithmetic — which is the part that can be subtly wrong.
 *
 * The algorithm: walk the items largest first, adding each to the current row
 * along the shorter side of the remaining space. Keep adding while the worst
 * aspect ratio in the row improves; when it would get worse, lay the row out and
 * start a new one. Rows placed along the shorter side is what keeps the
 * rectangles close to square, which is the whole point — a person can compare
 * two squares by eye and cannot compare two slivers.
 */

/** An axis-aligned rectangle in canvas pixels. */
export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** One item with the rectangle it was given. */
export interface Placed<T> extends Rect {
  item: T;
}

/**
 * Lay `items` out inside `bounds`, proportionally to `sizeOf`.
 *
 * Items are laid out in the order given — the caller sorts, because "largest
 * first" is the caller's business and sorting here would hide it. Items with a
 * size of zero still get a rectangle if any space is left over, because a
 * zero-byte file that vanishes from the map looks like a bug to whoever is
 * hunting for it; they are simply the last and thinnest.
 */
export function squarify<T>(
  items: readonly T[],
  sizeOf: (item: T) => number,
  bounds: Rect,
): Placed<T>[] {
  const placed: Placed<T>[] = [];
  if (items.length === 0 || bounds.w <= 0 || bounds.h <= 0) {
    return placed;
  }

  const weights = items.map((item) => Math.max(0, sizeOf(item)));
  const total = weights.reduce((sum, value) => sum + value, 0);

  // Everything is zero: there is no proportion to honour, so split evenly
  // rather than dividing by zero or drawing nothing.
  const area = bounds.w * bounds.h;
  const scale = total > 0 ? area / total : area / items.length;
  const scaled = weights.map((weight) => (total > 0 ? weight * scale : scale));

  let free: Rect = { ...bounds };
  let index = 0;

  while (index < scaled.length) {
    const shortSide = Math.min(free.w, free.h);
    const row: number[] = [];
    let rowSum = 0;

    // Grow the row while the squareness improves.
    while (index < scaled.length) {
      const next = scaled[index];
      const currentWorst = worstRatio(row, rowSum, shortSide);
      const nextWorst = worstRatio([...row, next], rowSum + next, shortSide);
      if (row.length > 0 && nextWorst > currentWorst) {
        break;
      }
      row.push(next);
      rowSum += next;
      index += 1;
    }

    const rowStart = index - row.length;
    free = placeRow(placed, items, row, rowSum, free, rowStart);

    // Numerically the free box can go to zero before the items do; whatever is
    // left gets a sliver rather than disappearing.
    if (free.w <= 0.5 || free.h <= 0.5) {
      for (; index < scaled.length; index += 1) {
        placed.push({ item: items[index], x: free.x, y: free.y, w: 0, h: 0 });
      }
      break;
    }
  }

  return placed;
}

/** Worst aspect ratio in a row laid along `side`. Lower is squarer. */
function worstRatio(row: readonly number[], sum: number, side: number): number {
  if (row.length === 0 || sum <= 0 || side <= 0) {
    return Number.POSITIVE_INFINITY;
  }
  const max = Math.max(...row);
  const min = Math.min(...row);
  const sideSquared = side * side;
  const sumSquared = sum * sum;
  // Both halves of the classic `worst` formula; zero-area members would make it
  // infinite, so they are treated as "no worse than the row already is".
  const worstMax = min > 0 ? (sideSquared * max) / sumSquared : 0;
  const worstMin = min > 0 ? sumSquared / (sideSquared * min) : 0;
  return Math.max(worstMax, worstMin);
}

/** Place one finished row and return the space that is left. */
function placeRow<T>(
  placed: Placed<T>[],
  items: readonly T[],
  row: readonly number[],
  rowSum: number,
  free: Rect,
  startIndex: number,
): Rect {
  const horizontal = free.w >= free.h;
  // Thickness of the band this row occupies, across the long side.
  const thickness = horizontal
    ? clamp(rowSum / free.h, 0, free.w)
    : clamp(rowSum / free.w, 0, free.h);
  const along = horizontal ? free.h : free.w;

  let offset = 0;
  for (let i = 0; i < row.length; i += 1) {
    const share = rowSum > 0 ? (row[i] / rowSum) * along : along / row.length;
    placed.push(
      horizontal
        ? { item: items[startIndex + i], x: free.x, y: free.y + offset, w: thickness, h: share }
        : { item: items[startIndex + i], x: free.x + offset, y: free.y, w: share, h: thickness },
    );
    offset += share;
  }

  return horizontal
    ? { x: free.x + thickness, y: free.y, w: free.w - thickness, h: free.h }
    : { x: free.x, y: free.y + thickness, w: free.w, h: free.h - thickness };
}

function clamp(value: number, low: number, high: number): number {
  return Math.min(high, Math.max(low, value));
}

/** The rectangle at `(x, y)`, or null. Later rectangles win, as when painting. */
export function hitTest<T>(
  placed: readonly Placed<T>[],
  x: number,
  y: number,
): Placed<T> | null {
  for (let i = placed.length - 1; i >= 0; i -= 1) {
    const rect = placed[i];
    if (x >= rect.x && x <= rect.x + rect.w && y >= rect.y && y <= rect.y + rect.h) {
      return rect;
    }
  }
  return null;
}

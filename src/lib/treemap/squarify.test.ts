/**
 * Geometry tests for the squarified layout.
 *
 * Run with `npm run test:ui`. They use `node:test` and nothing else — Node
 * strips the types itself, so proving the arithmetic costs the project no new
 * dependency and no new build step.
 *
 * What is worth asserting here is exactly what a picture cannot show you: that
 * the rectangles tile the box with no gaps and no overlaps, and that their areas
 * are proportional. A treemap that is subtly out by a few percent looks
 * completely convincing.
 */
import assert from "node:assert/strict";
import { test } from "node:test";

import { hitTest, squarify, type Rect } from "./squarify.ts";

const BOX: Rect = { x: 0, y: 0, w: 600, h: 400 };

function areas(sizes: number[], bounds: Rect = BOX): number[] {
  return squarify(sizes, (n) => n, bounds).map((cell) => cell.w * cell.h);
}

test("an empty list produces no rectangles", () => {
  assert.deepEqual(squarify([], (n: number) => n, BOX), []);
});

test("one item fills the whole box", () => {
  const [cell] = squarify([42], (n) => n, BOX);
  assert.equal(cell.x, 0);
  assert.equal(cell.y, 0);
  assert.equal(cell.w, 600);
  assert.equal(cell.h, 400);
});

test("areas are proportional to the sizes", () => {
  const sizes = [50, 30, 20];
  const total = BOX.w * BOX.h;
  const measured = areas(sizes);
  for (let i = 0; i < sizes.length; i += 1) {
    const expected = (sizes[i] / 100) * total;
    assert.ok(
      Math.abs(measured[i] - expected) < 1,
      `item ${i}: expected ~${expected}, got ${measured[i]}`,
    );
  }
});

test("the rectangles add up to the whole box", () => {
  const sizes = [400, 220, 180, 90, 60, 30, 12, 5, 2, 1];
  const total = areas(sizes).reduce((sum, area) => sum + area, 0);
  assert.ok(
    Math.abs(total - BOX.w * BOX.h) < 1,
    `the layout covered ${total} of ${BOX.w * BOX.h}`,
  );
});

test("no two rectangles overlap", () => {
  const placed = squarify(
    [400, 220, 180, 90, 60, 30, 12, 5, 2, 1],
    (n) => n,
    BOX,
  );
  for (let i = 0; i < placed.length; i += 1) {
    for (let j = i + 1; j < placed.length; j += 1) {
      const a = placed[i];
      const b = placed[j];
      const separated =
        a.x + a.w <= b.x + 0.001 ||
        b.x + b.w <= a.x + 0.001 ||
        a.y + a.h <= b.y + 0.001 ||
        b.y + b.h <= a.y + 0.001;
      assert.ok(separated, `rectangles ${i} and ${j} overlap`);
    }
  }
});

test("every rectangle stays inside the box", () => {
  for (const cell of squarify([9, 8, 7, 6, 5, 4, 3, 2, 1], (n) => n, BOX)) {
    assert.ok(cell.x >= -0.001 && cell.y >= -0.001, "outside the top left");
    assert.ok(
      cell.x + cell.w <= BOX.w + 0.001 && cell.y + cell.h <= BOX.h + 0.001,
      "outside the bottom right",
    );
  }
});

test("squarifying keeps aspect ratios sane rather than slicing", () => {
  // Sixteen equal items in a square box should come out roughly square. A naive
  // slice-and-dice layout would produce sixteen 25x400 slivers.
  const placed = squarify(Array<number>(16).fill(1), (n) => n, {
    x: 0,
    y: 0,
    w: 400,
    h: 400,
  });
  for (const cell of placed) {
    const ratio = Math.max(cell.w / cell.h, cell.h / cell.w);
    assert.ok(ratio < 3, `aspect ratio ${ratio.toFixed(2)} is a sliver`);
  }
});

test("zero-sized items still get a rectangle so they can be found", () => {
  const placed = squarify([100, 0, 0], (n) => n, BOX);
  assert.equal(placed.length, 3, "nothing may silently vanish from the map");
});

test("all-zero sizes split the box evenly instead of dividing by zero", () => {
  const measured = areas([0, 0, 0, 0]);
  const expected = (BOX.w * BOX.h) / 4;
  for (const area of measured) {
    assert.ok(
      Math.abs(area - expected) < 1,
      `expected ~${expected}, got ${area}`,
    );
  }
});

test("a zero-sized box produces nothing rather than NaN", () => {
  assert.deepEqual(squarify([1, 2], (n) => n, { x: 0, y: 0, w: 0, h: 0 }), []);
});

test("hit testing finds the rectangle under a point", () => {
  const placed = squarify([70, 30], (n) => n, BOX);
  const first = placed[0];
  const hit = hitTest(placed, first.x + first.w / 2, first.y + first.h / 2);
  assert.equal(hit?.item, 70);
  assert.equal(hitTest(placed, -5, -5), null);
});

test("four hundred rectangles lay out in well under a frame", () => {
  const sizes = Array.from({ length: 400 }, (_, i) => 400 - i);
  const started = performance.now();
  for (let run = 0; run < 20; run += 1) {
    squarify(sizes, (n) => n, BOX);
  }
  const perLayout = (performance.now() - started) / 20;
  console.log(`  squarify(400 rects): ${perLayout.toFixed(3)} ms per layout`);
  assert.ok(
    perLayout < 16,
    `a layout took ${perLayout.toFixed(2)} ms, which would drop frames`,
  );
});

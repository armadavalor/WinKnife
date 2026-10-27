/** The procedural sky. Run with `npm run test:ui`. */
import assert from "node:assert/strict";
import { test } from "node:test";
import { ASCII_RAMP, LOOP_SECONDS, PERIOD, cloudDensity, hillTop, isBackdrop, rampChar, toneAt } from "./sky.ts";

test("clouds loop: the field repeats exactly every PERIOD along x", () => {
  for (const [u, v] of [[0.3, 1.1], [2.7, 0.4], [5.55, 3.2]] as const) {
    assert.ok(Math.abs(cloudDensity(u, v) - cloudDensity(u + PERIOD, v)) < 1e-9);
  }
});

test("density stays within 0..1", () => {
  for (let i = 0; i < 500; i++) {
    const d = cloudDensity(i * 0.37, i * 0.11);
    assert.ok(d >= 0 && d <= 1, `d=${d}`);
  }
});

test("the hill is low and round: always in the lower half", () => {
  for (let x = 0; x <= 1; x += 0.05) {
    const top = hillTop(x);
    assert.ok(top > 0.6 && top < 0.9, `top=${top}`);
  }
});

test("below the hill line is hill; above is sky or cloud; b is 0..1", () => {
  for (let x = 0.05; x < 1; x += 0.1) {
    assert.equal(toneAt(x, 0.97, 1.6, 5).kind, "hill");
    for (let y = 0.02; y < 0.5; y += 0.1) {
      const tone = toneAt(x, y, 1.6, 5);
      assert.notEqual(tone.kind, "hill");
      assert.ok(tone.b >= 0 && tone.b <= 1);
    }
  }
});

test("there are clouds somewhere in the sky", () => {
  let clouds = 0;
  for (let x = 0; x < 1; x += 0.02) {
    for (let y = 0; y < 0.6; y += 0.02) if (toneAt(x, y, 1.6, 0).kind === "cloud") clouds++;
  }
  assert.ok(clouds > 20, `clouds=${clouds}`);
});

test("the drift loops: the sky at t and t + LOOP_SECONDS is the same", () => {
  for (let x = 0.03; x < 1; x += 0.07) {
    for (let y = 0.05; y < 0.6; y += 0.07) {
      const a = toneAt(x, y, 1.6, 12);
      const b = toneAt(x, y, 1.6, 12 + LOOP_SECONDS);
      assert.equal(a.kind, b.kind);
      assert.ok(Math.abs(a.b - b.b) < 1e-6);
    }
  }
});

test("clouds are dense and bright, with sparse dark sky between", () => {
  let cloud = 0;
  let sum = 0;
  let sky = 0;
  let skySum = 0;
  for (let x = 0; x < 1; x += 0.01) {
    for (let y = 0; y < 0.6; y += 0.01) {
      const tone = toneAt(x, y, 1.6, 40);
      if (tone.kind === "cloud") {
        cloud++;
        sum += tone.b;
      } else if (tone.kind === "sky") {
        sky++;
        skySum += tone.b;
      }
    }
  }
  assert.ok(cloud / (cloud + sky) > 0.12, "clouds cover a visible share of the sky");
  assert.ok(sum / cloud > 0.5, "cloud fill is dense");
  assert.ok(skySum / sky < 0.3, "sky between clouds stays dark");
});

test("the ascii ramp runs from blank to densest", () => {
  assert.equal(rampChar(0), " ");
  assert.equal(rampChar(1), ASCII_RAMP.at(-1));
  assert.equal(rampChar(7), ASCII_RAMP.at(-1));
});

test("backdrop values", () => {
  assert.equal(isBackdrop("halftone"), true);
  assert.equal(isBackdrop("light"), true);
  assert.equal(isBackdrop("x"), false);
});

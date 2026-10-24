/**
 * Icon geometry vendored from Lucide (https://lucide.dev), lucide-static 1.57.0.
 *
 * Copied as data rather than installed as a package: the app must work with no
 * network, only a handful of glyphs are used, and one stroke style keeps every
 * icon consistent across the three themes. To add an icon, copy the child
 * elements of `icons/<name>.svg` from the lucide-static package.
 *
 * ISC License — Copyright (c) 2026 Lucide Icons and Contributors.
 * Permission to use, copy, modify, and/or distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
 * ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
 * OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 *
 * `power`, `chevron-right` and `info` derive from Feather:
 * MIT License — Copyright (c) 2013-present Cole Bemis. Permission is hereby
 * granted, free of charge, to any person obtaining a copy of this software and
 * associated documentation files (the "Software"), to deal in the Software
 * without restriction, including without limitation the rights to use, copy,
 * modify, merge, publish, distribute, sublicense, and/or sell copies of the
 * Software, and to permit persons to whom the Software is furnished to do so,
 * subject to the following conditions: The above copyright notice and this
 * permission notice shall be included in all copies or substantial portions of
 * the Software. THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND.
 */

export type IconNode = readonly [tag: string, attrs: Record<string, string>];

export const ICONS = {
  "layout-dashboard": [
    ["rect", {width: "7", height: "9", x: "3", y: "3", rx: "1"}],
    ["rect", {width: "7", height: "5", x: "14", y: "3", rx: "1"}],
    ["rect", {width: "7", height: "9", x: "14", y: "12", rx: "1"}],
    ["rect", {width: "7", height: "5", x: "3", y: "16", rx: "1"}],
  ],
  "flask-conical": [
    ["path", {d: "M14 2v6a2 2 0 0 0 .245.96l5.51 10.08A2 2 0 0 1 18 22H6a2 2 0 0 1-1.755-2.96l5.51-10.08A2 2 0 0 0 10 8V2"}],
    ["path", {d: "M6.453 15h11.094"}],
    ["path", {d: "M8.5 2h7"}],
  ],
  "brush-cleaning": [
    ["path", {d: "m16 22-1-4"}],
    ["path", {d: "M19 14a1 1 0 0 0 1-1v-1a2 2 0 0 0-2-2h-3a1 1 0 0 1-1-1V4a2 2 0 0 0-4 0v5a1 1 0 0 1-1 1H6a2 2 0 0 0-2 2v1a1 1 0 0 0 1 1"}],
    ["path", {d: "M19 14H5l-1.973 6.767A1 1 0 0 0 4 22h16a1 1 0 0 0 .973-1.233z"}],
    ["path", {d: "m8 22 1-4"}],
  ],
  "power": [
    ["path", {d: "M12 2v10"}],
    ["path", {d: "M18.4 6.6a9 9 0 1 1-12.77.04"}],
  ],
  "coffee": [
    ["path", {d: "M10 2v2"}],
    ["path", {d: "M14 2v2"}],
    ["path", {d: "M16 8a1 1 0 0 1 1 1v8a4 4 0 0 1-4 4H7a4 4 0 0 1-4-4V9a1 1 0 0 1 1-1h14a4 4 0 1 1 0 8h-1"}],
    ["path", {d: "M6 2v2"}],
  ],
  "folder-search": [
    ["path", {d: "M10.7 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v4.1"}],
    ["path", {d: "m21 21-1.9-1.9"}],
    ["circle", {cx: "17", cy: "17", r: "3"}],
  ],
  "shield-check": [
    ["path", {d: "M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"}],
    ["path", {d: "m9 12 2 2 4-4"}],
  ],
  "copy": [
    ["rect", {width: "14", height: "14", x: "8", y: "8", rx: "2", ry: "2"}],
    ["path", {d: "M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"}],
  ],
  "settings": [
    ["path", {d: "M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915"}],
    ["circle", {cx: "12", cy: "12", r: "3"}],
  ],
  "file": [
    ["path", {d: "M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"}],
    ["path", {d: "M14 2v5a1 1 0 0 0 1 1h5"}],
  ],
  "chart-pie": [
    ["path", {d: "M21 12c.552 0 1.005-.449.95-.998a10 10 0 0 0-8.953-8.951c-.55-.055-.998.398-.998.95v8a1 1 0 0 0 1 1z"}],
    ["path", {d: "M21.21 15.89A10 10 0 1 1 8 2.83"}],
  ],
  "lock-open": [
    ["rect", {width: "18", height: "11", x: "3", y: "11", rx: "2", ry: "2"}],
    ["path", {d: "M7 11V7a5 5 0 0 1 9.9-1"}],
  ],
  "rocket": [
    ["path", {d: "M12 15v5s3.03-.55 4-2c1.08-1.62 0-5 0-5"}],
    ["path", {d: "M4.5 16.5c-1.5 1.26-2 5-2 5s3.74-.5 5-2c.71-.84.7-2.13-.09-2.91a2.18 2.18 0 0 0-2.91-.09"}],
    ["path", {d: "M9 12a22 22 0 0 1 2-3.95A12.88 12.88 0 0 1 22 2c0 2.72-.78 7.5-6 11a22.4 22.4 0 0 1-4 2z"}],
    ["path", {d: "M9 12H4s.55-3.03 2-4c1.62-1.08 5 .05 5 .05"}],
  ],
  "app-window": [
    ["rect", {x: "2", y: "4", width: "20", height: "16", rx: "2"}],
    ["path", {d: "M10 4v4"}],
    ["path", {d: "M2 8h20"}],
    ["path", {d: "M6 4v4"}],
  ],
  "history": [
    ["path", {d: "M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"}],
    ["path", {d: "M3 3v5h5"}],
    ["path", {d: "M12 7v5l4 2"}],
  ],
  "chevron-right": [
    ["path", {d: "m9 18 6-6-6-6"}],
  ],
  "info": [
    ["circle", {cx: "12", cy: "12", r: "10"}],
    ["path", {d: "M12 16v-4"}],
    ["path", {d: "M12 8h.01"}],
  ],
  "folder": [
    ["path", {d: "M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"}],
  ],
  "list": [
    ["path", {d: "M3 5h.01"}],
    ["path", {d: "M3 12h.01"}],
    ["path", {d: "M3 19h.01"}],
    ["path", {d: "M8 5h13"}],
    ["path", {d: "M8 12h13"}],
    ["path", {d: "M8 19h13"}],
  ],
  "globe": [
    ["circle", {cx: "12", cy: "12", r: "10"}],
    ["path", {d: "M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20"}],
    ["path", {d: "M2 12h20"}],
  ],
} as const satisfies Record<string, readonly IconNode[]>;

export type IconName = keyof typeof ICONS;

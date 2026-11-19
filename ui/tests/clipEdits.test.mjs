import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import ts from "typescript";

const source = readFileSync(new URL("../src/lib/clipEdits.ts", import.meta.url), "utf8");
const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
const { splitAt, removeRange, presetCrop, segmentDuration, editHistory, initialHistory, NO_CROP, parseDraft, fadeGain, quietRanges } = await import(`data:text/javascript;base64,${Buffer.from(js).toString("base64")}`);

test("split twice and remove a boring middle section without losing edges", () => {
  let segments = splitAt([[0, 10]], 3);
  segments = splitAt(segments, 7);
  assert.deepEqual(segments, [[0, 3], [3, 7], [7, 10]]);
  segments = segments.filter((_, i) => i !== 1);
  assert.equal(segmentDuration(segments), 6);
  assert.deepEqual(segments, removeRange([[0, 10]], [3, 7]));
});

test("range removal across multiple segments preserves all remaining portions", () => {
  assert.deepEqual(removeRange([[0, 3], [5, 8], [9, 12]], [2, 10]), [[0, 2], [10, 12]]);
  assert.deepEqual(removeRange([[0, 1]], [0, 1]), []);
  assert.deepEqual(splitAt([[0, 1]], 0.001), [[0, 1]]);
});

test("portrait and square presets yield the requested aspect ratio", () => {
  for (const ratio of [1, 9 / 16, 16 / 9]) {
    const c = presetCrop(1920, 1080, ratio);
    assert.ok(Math.abs(1920 * (1 - c.left - c.right) / (1080 * (1 - c.top - c.bottom)) - ratio) < 1e-9);
  }
});

test("undo and redo restore both segments and crop; new edits discard redo", () => {
  const first = { segments: [[0, 2], [4, 6]], crop: NO_CROP };
  const second = { ...first, crop: presetCrop(1920, 1080, 1) };
  let h = editHistory(initialHistory(), { type: "change", value: first });
  h = editHistory(h, { type: "change", value: second });
  h = editHistory(h, { type: "undo" });
  assert.deepEqual(h.present, first);
  h = editHistory(h, { type: "redo" });
  assert.deepEqual(h.present, second);
  h = editHistory(h, { type: "undo" });
  h = editHistory(h, { type: "change", value: { ...first, segments: [[0, 1]] } });
  assert.equal(h.future.length, 0);
});

test("draft recipes validate source duration and reject malformed edits", () => {
  const saved = { version: 1, duration: 10, range: [1, 9], draft: { crop: NO_CROP, segments: [[7, 9], [1, 3]], audio: { fadeIn: .5, fadeOut: .5, normalize: true, denoise: false } } };
  assert.deepEqual(parseDraft(JSON.stringify(saved), 10), saved);
  assert.equal(parseDraft(JSON.stringify(saved), 11), null);
  assert.equal(parseDraft("{broken", 10), null);
  saved.draft.crop = { ...NO_CROP, left: .99 };
  assert.equal(parseDraft(JSON.stringify(saved), 10), null);
});

test("fade preview maps reordered source ranges into output time", () => {
  const effects = { fadeIn: 1, fadeOut: 1, normalize: false, denoise: false };
  assert.equal(fadeGain(7, [[7, 10], [0, 3]], effects), 0);
  assert.equal(fadeGain(7.5, [[7, 10], [0, 3]], effects), .5);
  assert.equal(fadeGain(2.5, [[7, 10], [0, 3]], effects), .5);
  assert.equal(fadeGain(3, [[7, 10], [0, 3]], effects), 0);
});
test("quiet suggestions keep short pauses and close trailing silence", () => {
  assert.deepEqual(quietRanges([.5,0,0,.5,0], 5), [[1,3],[4,5]]);
  assert.deepEqual(quietRanges([.5,0,.5], .3), []);
});

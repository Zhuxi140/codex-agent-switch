import assert from "node:assert/strict";
import test from "node:test";

import { normalizeRanges } from "../src/ranges.mjs";

test("normalizeRanges sorts and merges overlapping or adjacent closed ranges", () => {
  assert.deepEqual(
    normalizeRanges([
      [10, 10],
      [3, 4],
      [1, 2],
      [8, 9],
      [5, 7],
    ]),
    [[1, 10]],
  );
  assert.deepEqual(normalizeRanges([[1, 10], [3, 4], [20, 21]]), [
    [1, 10],
    [20, 21],
  ]);
});

test("normalizeRanges accepts an empty list and does not mutate input", () => {
  assert.deepEqual(normalizeRanges([]), []);

  const input = [[5, 6], [1, 2]];
  const snapshot = structuredClone(input);
  const result = normalizeRanges(input);

  assert.deepEqual(input, snapshot);
  assert.deepEqual(result, [[1, 2], [5, 6]]);
  assert.notEqual(result[0], input[1]);
});

test("normalizeRanges rejects malformed ranges", () => {
  for (const value of [
    null,
    {},
    [1, 2],
    [[1]],
    [[1, 2, 3]],
    [[2, 1]],
    [[1.5, 2]],
    [[1, Number.POSITIVE_INFINITY]],
    [[Number.MAX_SAFE_INTEGER + 1, Number.MAX_SAFE_INTEGER + 1]],
  ]) {
    assert.throws(() => normalizeRanges(value), TypeError);
  }
});

import assert from "node:assert/strict";
import test from "node:test";

import { subtractRanges } from "../src/ranges.mjs";

test("subtractRanges handles full, head, tail and middle subtraction", () => {
  assert.deepEqual(subtractRanges([[1, 10]], [[3, 4], [7, 20]]), [
    [1, 2],
    [5, 6],
  ]);
  assert.deepEqual(subtractRanges([[1, 5]], [[1, 2]]), [[3, 5]]);
  assert.deepEqual(subtractRanges([[1, 5]], [[4, 5]]), [[1, 3]]);
  assert.deepEqual(subtractRanges([[1, 5]], [[1, 5]]), []);
});

test("subtractRanges normalizes both inputs and spans multiple ranges", () => {
  assert.deepEqual(
    subtractRanges([[20, 25], [1, 5], [6, 10]], [[23, 30], [4, 7]]),
    [[1, 3], [8, 10], [20, 22]],
  );
  assert.deepEqual(subtractRanges([[5, 6], [1, 2]], []), [[1, 2], [5, 6]]);
});

test("subtractRanges does not mutate inputs and validates both lists", () => {
  const ranges = [[1, 10]];
  const exclusions = [[3, 4]];
  const rangesSnapshot = structuredClone(ranges);
  const exclusionsSnapshot = structuredClone(exclusions);

  assert.deepEqual(subtractRanges(ranges, exclusions), [[1, 2], [5, 10]]);
  assert.deepEqual(ranges, rangesSnapshot);
  assert.deepEqual(exclusions, exclusionsSnapshot);

  assert.throws(() => subtractRanges([[1, 2]], [[4, 3]]), TypeError);
  assert.throws(() => subtractRanges("invalid", []), TypeError);
  assert.throws(() => subtractRanges([], "invalid"), TypeError);
});

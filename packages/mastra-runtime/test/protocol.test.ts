import assert from "node:assert/strict";
import { test } from "node:test";

import { canonicalize } from "../src/jcs.js";
import { DEFAULT_LIMITS } from "../src/limits.js";
import { formatReport, runConformance } from "../src/conformance.js";
import { SchemaBundle, decodeFrameBytes, encodeFrame } from "../src/protocol.js";
import { parseStrictJson, StrictJsonError } from "../src/strict-json.js";

const bundle = SchemaBundle.load();

test("the repository schema bundle verifies and the TypeScript layer agrees with every owned case", () => {
  const report = runConformance(bundle);
  assert.equal(report.failures.length, 0, formatReport(report));
  assert.equal(report.total, 1002);
  assert.equal(report.checked, 899);
  assert.equal(report.passed, 899);
  assert.equal(report.skipped, 103);
  assert.deepEqual(report.skippedKinds, { data: 93, transcript: 10 });
});

test("strict JSON rejects duplicate keys, nonfinite numbers and the depth ceiling", () => {
  assert.throws(() => parseStrictJson('{"a":1,"a":2}', 8), StrictJsonError);
  assert.throws(() => parseStrictJson('{"a":1e400}', 8), StrictJsonError);
  assert.throws(() => parseStrictJson(`${"[".repeat(9)}0${"]".repeat(9)}`, 8), StrictJsonError);
  assert.deepEqual(parseStrictJson('{"b":2,"a":[1,true,null,"x"]}', 8), { b: 2, a: [1, true, null, "x"] });
});

test("decimal revisions stay strings and never become JavaScript numbers", () => {
  const raw = Buffer.from(
    `${JSON.stringify({
      documentId: "doc-A",
      revision: "9007199254740993",
    })}\n`,
    "utf8",
  );
  const parsed = parseStrictJson(raw.toString("utf8"), DEFAULT_LIMITS.jsonDepth) as { revision: string };
  assert.equal(parsed.revision, "9007199254740993");
  assert.equal(typeof parsed.revision, "string");
});

test("accepted frames round-trip through the encoder", () => {
  const frame = {
    protocolVersion: "1.0",
    kind: "request",
    requestId: "h:round",
    scope: { sessionId: "s", turnId: "t", workflowRunId: "w" },
    method: "turn.cancel",
    params: { reason: "user" },
  } as const;
  const decoded = decodeFrameBytes(encodeFrame(frame), "negotiated", "host", DEFAULT_LIMITS, bundle);
  assert.equal(decoded.kind, "frame");
  assert.deepEqual(decoded.kind === "frame" ? decoded.frame : null, frame);
});

test("RFC 8785 vectors match the pinned canonicalizer behavior", () => {
  assert.equal(
    canonicalize([333333333.33333329, 1e30, 4.5, 2e-3, 0.000000000000000000000000001, -0]),
    "[333333333.3333333,1e+30,4.5,0.002,1e-27,0]",
  );
  assert.equal(
    canonicalize({ "€": "Euro", "\r": "CR", "\uFB33": "Hebrew", "1": "One", "😀": "Emoji", "\u0080": "Control", "ö": "Latin" }),
    '{"\\r":"CR","1":"One","\u0080":"Control","ö":"Latin","€":"Euro","😀":"Emoji","\uFB33":"Hebrew"}',
  );
  assert.equal(canonicalize({ revision: "18446744073709551615" }), '{"revision":"18446744073709551615"}');
});

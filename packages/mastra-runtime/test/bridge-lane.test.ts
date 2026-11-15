import assert from "node:assert/strict";
import { PassThrough } from "node:stream";
import { test } from "node:test";

import { bridgeLane } from "../src/bridge-lane.js";

function tick(): Promise<void> {
  return new Promise((resolve) => setImmediate(resolve));
}

test("a closed turn channel discards its partial line before the next MCP session", async () => {
  const stream = new PassThrough();
  const lane = bridgeLane(stream, stream);
  const first: string[] = [];
  const channel = lane.channel();
  channel.receive(
    (line) => first.push(line),
    () => undefined,
  );
  stream.write("partial-without-newline");
  await tick();
  assert.deepEqual(first, []);
  channel.close("turn finished");

  const second: string[] = [];
  const next = lane.channel();
  next.receive(
    (line) => second.push(line),
    () => undefined,
  );
  stream.write("hello\n");
  await tick();
  assert.deepEqual(second, ["hello"], "the previous turn's partial bytes are not prepended");
  lane.close("test ended");
});

test("the lane never replays a complete line to an inactive turn", async () => {
  const stream = new PassThrough();
  const lane = bridgeLane(stream, stream);
  const channel = lane.channel();
  const received: string[] = [];
  channel.receive(
    (line) => received.push(line),
    () => undefined,
  );
  stream.write("one\n");
  await tick();
  channel.close("turn finished");
  stream.write("unclaimed\n");
  await tick();
  assert.deepEqual(received, ["one"]);
  lane.close("test ended");
});

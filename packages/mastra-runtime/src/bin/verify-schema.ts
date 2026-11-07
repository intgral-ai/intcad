#!/usr/bin/env node
import { formatReport, runConformance } from "../conformance.js";
import { SchemaBundle } from "../protocol.js";

const bundle = SchemaBundle.load();
const report = runConformance(bundle);
process.stdout.write(`${formatReport(report)}\n`);
if (report.failures.length > 0) {
  process.exitCode = 1;
}

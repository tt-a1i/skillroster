#!/usr/bin/env node
import {
  closeSync,
  constants,
  fstatSync,
  lstatSync,
  openSync,
  readSync,
} from "node:fs";

import {
  PilotError,
  renderPilotReport,
  summarizePilot,
} from "../tests/harness/roster-recommendation-pilot/pilot.mjs";

const failUsage = (message) => { throw new PilotError("invalid_arguments", message); };
const MAX_INPUT_BYTES = 1024 * 1024;

const parseArguments = (arguments_) => {
  const [command, ...rest] = arguments_;
  if (command !== "summarize" && command !== "report") {
    failUsage("expected summarize or report");
  }
  if (rest.length !== 2 || rest[0] !== "--input" || !rest[1]) {
    failUsage("expected exactly --input <ledger.json>");
  }
  return { command, input: rest[1] };
};

const readLedger = (input) => {
  let descriptor;
  try {
    const pathIdentity = lstatSync(input);
    if (!pathIdentity.isFile()) throw new PilotError("unreadable_input", "pilot ledger must be a regular file");
    descriptor = openSync(input, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
    const openedIdentity = fstatSync(descriptor);
    if (
      !openedIdentity.isFile()
      || (pathIdentity.ino !== 0 && openedIdentity.ino !== 0 && (
        pathIdentity.dev !== openedIdentity.dev || pathIdentity.ino !== openedIdentity.ino
      ))
    ) {
      throw new PilotError("unreadable_input", "pilot ledger identity changed before read");
    }
    if (openedIdentity.size > MAX_INPUT_BYTES) {
      throw new PilotError("input_too_large", `pilot ledger exceeds ${MAX_INPUT_BYTES} bytes`);
    }
    const buffer = Buffer.allocUnsafe(MAX_INPUT_BYTES + 1);
    let length = 0;
    while (length <= MAX_INPUT_BYTES) {
      const bytesRead = readSync(descriptor, buffer, length, buffer.length - length, null);
      if (bytesRead === 0) break;
      length += bytesRead;
    }
    if (length > MAX_INPUT_BYTES) {
      throw new PilotError("input_too_large", `pilot ledger exceeds ${MAX_INPUT_BYTES} bytes`);
    }
    return JSON.parse(buffer.subarray(0, length).toString("utf8"));
  } catch (error) {
    if (error instanceof PilotError) throw error;
    throw new PilotError("unreadable_input", "pilot ledger must be readable JSON");
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
};

const main = () => {
  const { command, input } = parseArguments(process.argv.slice(2));
  const ledger = readLedger(input);
  const summary = summarizePilot(ledger);
  if (command === "report") process.stdout.write(renderPilotReport(summary));
  else process.stdout.write(`${JSON.stringify(summary, null, 2)}\n`);
};

try {
  main();
} catch (error) {
  process.stderr.write(`${error.code ?? "pilot_error"}: ${error.message}\n`);
  process.exitCode = 1;
}

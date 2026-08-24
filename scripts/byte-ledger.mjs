#!/usr/bin/env node
import { runCli } from "../tests/harness/byte-ledger/ledger.mjs";

runCli(process.argv.slice(2)).catch((error) => {
  process.stderr.write(`${error.code ?? "error"}: ${error.message}\n`);
  process.exitCode = 1;
});

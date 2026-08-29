#!/usr/bin/env bash
set -euo pipefail

node --check tests/harness/pi-cold-routing/bounds.mjs
node --check tests/harness/pi-cold-routing/runner.mjs
node --check tests/harness/codex-cold-routing/driver.mjs
node --check tests/harness/byte-ledger/ledger.mjs
node --check scripts/byte-ledger.mjs
node --check scripts/recommendation-pilot.mjs
node --check tests/harness/roster-recommendation-pilot/pilot.mjs
node --experimental-strip-types --check tests/harness/pi-cold-routing/gate.ts
node --test \
  tests/harness/pi-cold-routing/*.test.mjs \
  tests/harness/codex-cold-routing/*.test.mjs \
  tests/harness/byte-ledger/*.test.mjs \
  tests/harness/roster-recommendation-pilot/*.test.mjs

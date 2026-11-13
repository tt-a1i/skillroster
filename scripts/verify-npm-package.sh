#!/usr/bin/env bash
set -euo pipefail
node --check npm/bin/skillroster.cjs
node --test tests/npm/*.test.cjs

#!/usr/bin/env bash
# Explicit synthetic measurement, not a timing gate or whole-CLI benchmark.
set -euo pipefail
cd "$(dirname "$0")/.."
case "$(uname -s)" in
  Darwin) timing=(-l) ;;
  Linux) timing=(-v) ;;
  *) echo 'Peak-RSS measurement requires macOS or Linux /usr/bin/time.' >&2; exit 1 ;;
esac
executable="$(cargo test --locked --release --lib --no-run --message-format=json | node -e '
let input = "";
process.stdin.on("data", chunk => input += chunk);
process.stdin.on("end", () => {
  const artifacts = input.trim().split("\n").map(line => JSON.parse(line))
    .filter(row => row.reason === "compiler-artifact" && row.target.name === "skillroster"
      && row.target.kind.includes("lib") && row.profile.test && row.executable);
  if (artifacts.length !== 1) process.exit(1);
  process.stdout.write(artifacts[0].executable);
});
')"
rustc --version
uname -sm
for count in 193 1000 5000; do
  SKILLROSTER_BENCH_SKILLS="$count" /usr/bin/time "${timing[@]}" "$executable" \
    --exact query::tests::semantic_overlap_scale_measurement --ignored --nocapture
done

#!/usr/bin/env python3
"""Measure ordinary Find using disposable synthetic Skills; no timing gates."""

import argparse
import json
import os
from pathlib import Path
import statistics
import subprocess
import tempfile
import time


def positive(value):
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("must be positive")
    return number


def nonnegative(value):
    number = int(value)
    if number < 0:
        raise argparse.ArgumentTypeError("must be zero or positive")
    return number


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--binary", type=Path,
        default=Path(__file__).resolve().parent.parent / "target/debug/skillroster",
    )
    parser.add_argument("--skills", type=positive, default=100)
    parser.add_argument("--support-bytes", type=nonnegative, default=1048576)
    parser.add_argument("--trials", type=positive, default=5)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)

    with tempfile.TemporaryDirectory(prefix="skillroster-find-benchmark-") as directory:
        root = Path(directory)
        home = root / "home"
        source = root / "skills"
        home.mkdir()
        source.mkdir()
        env = dict(os.environ, HOME=str(home), CODEX_HOME=str(home / ".codex"))
        command = [
            str(binary), "--home", str(home), "--state-dir", str(root / "state"),
            "--source-root", str(source),
        ]

        def run(arguments):
            started = time.perf_counter()
            process = subprocess.run(
                command + arguments, capture_output=True, check=True, cwd=root, env=env,
            )
            return (time.perf_counter() - started) * 1000, process.stdout

        entrypoint_bytes = 0
        for index in range(args.skills):
            name = f"utility-{index:03d}"
            skill = source / name
            references = skill / "references"
            references.mkdir(parents=True)
            entrypoint = (
                f"---\nname: {name}\n"
                f"description: Common utility automation for task number {index:03d}.\n"
                f"---\n# Utility {index:03d}\nUse this skill to perform utility operations.\n"
            ).encode()
            (skill / "SKILL.md").write_bytes(entrypoint)
            entrypoint_bytes += len(entrypoint)
            pattern = f"Reference content for skill {index:03d}.\n".encode()
            repeats, remainder = divmod(args.support_bytes, len(pattern))
            (references / "manual.txt").write_bytes(pattern * repeats + pattern[:remainder])

        _, version = run(["--version"])
        scan_ms, scan_output = run(["scan", "--json"])
        if not json.loads(scan_output)["ok"]:
            raise RuntimeError("Scan did not succeed")
        queries = {}
        for label, task in [("exact", f"utility-{args.skills // 2:03d}"), ("broad", "utility")]:
            trials = []
            for _ in range(args.trials):
                duration, output = run(["find", task, "--json"])
                document = json.loads(output)
                if not document["ok"]:
                    raise RuntimeError("Find did not succeed")
                matches = document["result"]["matches"]
                trials.append({
                    "milliseconds": duration,
                    "response_bytes": len(output),
                    "match_count": len(matches),
                    "content_choice_count": sum("content_choice" in match for match in matches),
                    "action_count": len(document["suggested_actions"]),
                })
            queries[label] = {
                "task": task,
                "median_milliseconds": statistics.median(t["milliseconds"] for t in trials),
                "trials": trials,
            }
        print(json.dumps({
            "binary": str(binary),
            "cli_version": version.decode().strip(),
            "fixture": {
                "skills": args.skills,
                "support_bytes_per_skill": args.support_bytes,
                "total_support_bytes": args.skills * args.support_bytes,
                "total_entrypoint_bytes": entrypoint_bytes,
            },
            "scan_milliseconds": scan_ms,
            "queries": queries,
        }, separators=(",", ":")))


if __name__ == "__main__":
    main()

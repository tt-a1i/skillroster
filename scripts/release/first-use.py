#!/usr/bin/env python3
"""Exercise an archive binary after installing it into an isolated PATH."""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile


repo = Path(__file__).resolve().parents[2]
binary = Path(sys.argv[1]).resolve()
cli_version = re.search(r'^version = "([^"]+)"', (repo / "Cargo.toml").read_text(), re.M)[1]
bootstrap_version = re.search(
    r'bootstrap-version: "([^"]+)"',
    (repo / "skill/skillroster/SKILL.md").read_text(),
)[1]

with tempfile.TemporaryDirectory(prefix="skillroster-first-use-") as directory:
    root = Path(directory)
    install = root / "install/bin"
    install.mkdir(parents=True)
    installed = install / binary.name
    shutil.copy2(binary, installed)
    env = dict(os.environ, PATH=str(install) + os.pathsep + os.environ.get("PATH", ""))
    selected = Path(shutil.which(binary.name, path=env["PATH"])).resolve()
    assert selected == installed.resolve()
    # Bind the selected PATH entry explicitly; Windows process lookup does not
    # use a child's env PATH to resolve an unqualified executable name.
    version = subprocess.check_output([str(selected), "--version"], env=env, text=True).strip()
    assert version == f"skillroster {cli_version}", version

    for scenario in ("empty", "small", "duplicate", "large"):
        home, state = root / scenario / "home", root / scenario / "state"
        home.mkdir(parents=True)
        scenario_env = dict(env, HOME=str(home), USERPROFILE=str(home), CODEX_HOME=str(home / ".codex"),
                            XDG_CONFIG_HOME=str(home / ".config"), APPDATA=str(home / "AppData/Roaming"),
                            LOCALAPPDATA=str(home / "AppData/Local"))

        def run(*args, request=None):
            output = subprocess.run(
                [str(selected), "--home", str(home), "--state-dir", str(state), "--json", *args],
                input=None if request is None else json.dumps(request),
                env=scenario_env, text=True, capture_output=True, check=True,
            )
            envelope = json.loads(output.stdout)
            assert envelope["schema_version"] == 1 and envelope["ok"], envelope
            return envelope["result"]

        if scenario != "empty":
            for index in range(55 if scenario == "large" else 3):
                name, description = [("review", "Review pull requests for correctness, security, regressions, and test coverage"),
                                     ("translate", "Translate English and Chinese documents with accurate terminology and natural phrasing"),
                                     ("diagram", "Draw architecture diagrams showing components, connections, data flows, and boundaries")][index] if index < 3 else (f"extra-{index:02}", f"Special fixture capability {index}")
                skill = home / ".codex/skills" / name
                skill.mkdir(parents=True)
                skill.joinpath("SKILL.md").write_text(
                    f"---\nname: {name}\ndescription: {description}\n"
                    f"source: fixture/{name}\nversion: 1\n---\n", encoding="utf-8",
                )
        if scenario == "duplicate":
            shutil.copytree(home / ".codex/skills/review", home / ".claude/skills/review")
        run("scan", "--summary")
        report = run("report")
        assert report["native_visibility"] is not None
        expected = {"empty": "inventory_empty", "small": "no_review_items"}.get(scenario, "review_available")
        assert report["overview"]["state"] == expected, report["overview"]
        assert run()["pending_plan_count"] == 0
        setup_check = run("setup", "--check")
        assert setup_check["cli_version"] == cli_version
        assert setup_check["bootstrap_content_version"] == bootstrap_version
        assert setup_check["plan_id"] is None
        assert run()["pending_plan_count"] == 0
        assert len(run("report", "--full")["findings"]) == report["finding_count"]

        if scenario == "duplicate":
            finding = next(f for f in report["findings"] if f["kind"] == "exact_duplicate_placements")
            detail = run("report", "--finding", finding["id"])
            request = detail["planning"]["plan_request_template"]
            # This synthetic trial explicitly chooses the fixture's first exact-content source.
            request["finding_library_changes"][0]["canonical_placement_id"] = detail["planning"]["canonical_candidates"][0]["placement_id"]
            request["finding_library_changes"][0]["requested_state"] = "managed"
            plan = run("plan", "--stdin", request=request)
            impact = plan["impact"]["items"][0]
            assert impact["before"]["physical_source_count"] == 2
            assert impact["after"]["physical_source_count"] == 1
            assert impact["before"]["default_exposed_placement_count"] == impact["after"]["default_exposed_placement_count"] == 2
            assert impact["replacement_path_count"] == 1
            assert len(impact["affected_agents"]) == 2
            assert plan["reversible"] and plan["canonical_deletion_count"] == 0
        if scenario == "large":
            finding = next(f for f in report["findings"] if f["kind"] == "large_default_roster")
            detail = run("report", "--finding", finding["id"])
            selection = detail["planning"]["selection_evidence"]
            assert selection["positive_signal_core_count"] == 0
            assert selection["stable_fallback_core_count"] == 50
            assert selection["keep_current_roster_available"]
            assert selection["core_budget_is_user_choice"]
            # Inspection alone must not create the name-ordered reduction Plan.
            assert run()["pending_plan_count"] == 0
        if scenario == "small":
            setup = run("setup")
            receipt = run("apply", setup["plan_id"])
            assert receipt["verification"] == "passed"
            run("scan", "--summary")
            assert run("setup", "--check")["state"] == "up_to_date"
            package = home / ".codex/skills/skillroster"
            for source in (repo / "skill/skillroster").rglob("*.md"):
                target = package / source.relative_to(repo / "skill/skillroster")
                assert target.read_text(encoding="utf-8") == source.read_text(encoding="utf-8")
            assert run("undo", receipt["receipt_id"])["verification"] == "passed"
            assert not package.exists()
        print(f"first-use: {scenario} passed ({version}; Bootstrap {bootstrap_version})")

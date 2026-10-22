# Codex native discovery and visibility gap

Inspected 2026-10-22. Research handoff only; implementation and product-spec changes are not part of this note.

## Evidence boundary

- Installed CLI: `codex-cli 0.163.0`, executable `/opt/homebrew/bin/codex`.
- Native test: that executable's local `app-server --stdio`, initialized over JSON-RPC, then `skills/list` with an isolated project CWD and `forceReload: true`. No model turn or account API was called.
- Comparison binary: installed `skillroster 1.8.46`. Its scan result agrees with the inspected repository implementation. It was not a fresh build of the working tree.
- Both tools used temporary fixture directories. Codex HOME, CODEX_HOME, and XDG_CONFIG_HOME pointed into the fixture; SkillRoster used explicit `--home` and `--state-dir`. No user-home inventory or auth content was read.
- This establishes CLI/app-server behavior, not the installed desktop application's version or an active conversation's catalog. The standard `/Applications/Codex.app` binary path was absent.

Primary sources fetched during investigation:

1. [OpenAI Build skills](https://developers.openai.com/codex/skills/): documents the user `.agents/skills` root, symlink discovery, and path-based disabling through `skills.config`.
2. [OpenAI Config basics](https://developers.openai.com/codex/config-basic/): describes configuration layers and project trust. Its current documentation is not itself a version-pinned description of 0.163.0.
3. [OpenAI Configuration reference](https://developers.openai.com/codex/config-reference/): configuration schema reference; consult the current field definitions when implementing.
4. The installed 0.163.0 executable and generated protocol types are primary implementation evidence for the native results below. No upstream source commit was pinned in this investigation.

## Safe fixture and reproduction

Use `$FIXTURE` below as an arbitrary new temporary directory, never the real user home:

```text
$FIXTURE/
  home/
    .agents/skills/native-user/SKILL.md
    .codex/
      config.toml
      skills/legacy-user/SKILL.md
  project/
```

Both manifests contain valid frontmatter, with distinct names `native-user` and `legacy-user`, description `Probe fixture only`, and a short body. Configuration:

```toml
[[skills.config]]
path = "/absolute/fixture/home/.agents/skills/native-user/SKILL.md"
enabled = false
```

Start the native app-server with HOME=`$FIXTURE/home`, CODEX_HOME=`$FIXTURE/home/.codex`, XDG_CONFIG_HOME=`$FIXTURE/home/.config`, and CWD=`$FIXTURE/project`. Send `initialize`, notification `initialized`, then:

```json
{"id":2,"method":"skills/list","params":{"cwds":["/absolute/fixture/project"],"forceReload":true}}
```

Observed records, with fixture-specific paths omitted:

```json
{"name":"legacy-user","scope":"user","enabled":true,"pluginId":null}
{"name":"native-user","scope":"user","enabled":false,"pluginId":null}
```

The app-server may populate bundled system Skills under its isolated CODEX_HOME; do not assert total inventory counts. Assert the two named fixture records instead.

Run SkillRoster with that fixture home and a fresh temporary state directory:

```sh
skillroster --home "$FIXTURE/home" --state-dir "$FIXTURE/state" --json scan
```

Then repeat with a different fresh state directory and explicit root:

```sh
skillroster --home "$FIXTURE/home" --state-dir "$FIXTURE/explicit-state" \
  --root "codex=$FIXTURE/home/.agents/skills" --json scan
```

Read persisted `placements` or `scan_payloads` in the fixture database to inspect exact exposure facts. The JSON Scan response itself is a bounded summary, not the complete placement inventory.

Local reproduction script retained for the next implementation session: `/private/tmp/skillroster-codex-discovery-probe.py`. It creates fresh temporary directories and prints their location. Additional investigation artifacts are `/private/tmp/skillroster-codex-discovery-scan.json`, `/private/tmp/skillroster-codex-discovery-explicit-scan.json`, and fixture-only state directories `/private/tmp/skillroster-codex-discovery-state` and `/private/tmp/skillroster-codex-discovery-explicit-state`. Temporary artifacts are convenience evidence, not durable project dependencies.

## Two distinct gaps

### 1. Native user root omitted

`src/harness.rs::known_agent_roots` lists only `.codex/skills` for Codex. Default fixture scan therefore inventories `legacy-user` but omits `native-user` altogether. Native 0.163.0 inventories both. Retain legacy discovery: the installed native version still supports `.codex/skills`, even though the current skills guide presents `.agents/skills` as the user location.

This gap concerns discovery, independently of whether a discovered Skill is enabled.

### 2. Disabled native Skill reported as exposed

Adding `.agents/skills` with existing `--root` support makes `native-user` appear in the persisted Snapshot with:

```json
{"default_exposed":true,"owned_by_agent":true,"mutation_scope":"mutable","governable":true}
```

Native 0.163.0 reports the same entrypoint disabled. This is already reproducible without changing automatic root discovery.

`src/scan.rs::default_exposure_for_candidate` checks Codex directory structure but does not apply native Skill configuration. Existing `src/claude_visibility.rs::observe` filters to Claude Code only. Codex plugin enablement parsing in `scan.rs` is a different control: plugin settings do not implement per-Skill `skills.config` visibility.

The current product spec explicitly documents Claude's local visibility projection. Extending it to Codex requires a stated new contract; it must not silently relabel Claude evidence as generic cross-Agent evidence.

## Verified native user-Skill semantics

All results below are from the same installed **Codex 0.163.0** and isolated no-model `skills/list` probes. Each case starts a new app-server with a fresh HOME/CODEX_HOME. The fixture includes native and legacy user Skills plus `.agents/skills/linked-user`, a symlink to an existing fixture-only source directory. Native output uses the source entrypoint's canonical path. These are version-specific observations, not a claim about every future release.

| User `skills.config` input | Native result |
| --- | --- |
| Absolute native `SKILL.md`, `enabled=false` | Native user Skill disabled |
| Absolute legacy `.codex/skills/.../SKILL.md`, false | Legacy user Skill disabled |
| Absolute package directory, either root, false | Skill remains enabled; directory is not equivalent to entrypoint |
| `../.agents/skills/native-user/SKILL.md`, false | Disabled; relative to the user config's `.codex` directory |
| `.agents/skills/native-user/SKILL.md`, false | Remains enabled; not resolved relative to HOME |
| `../home/.agents/skills/native-user/SKILL.md`, false | Remains enabled; not resolved relative to CWD |
| `~/.agents/skills/native-user/SKILL.md`, false | Disabled; fixture HOME expansion works |
| Same entrypoint twice, false then true | Enabled |
| Same entrypoint twice, true then false | Disabled; last entry wins within this user file |
| Symlink lexical `.../linked-user/SKILL.md`, false | Linked Skill disabled |
| Canonical source `.../source/shared/SKILL.md`, false | Same linked Skill disabled |
| Symlink entrypoint false, canonical entrypoint true | Enabled; aliases share the last-wins identity in this case |
| Canonical entrypoint true, symlink entrypoint false | Disabled |
| Symlink package directory or canonical package directory, false | Linked Skill remains enabled |
| Entry missing `path`, false | Valid load; fixture Skills remain enabled |
| Entry missing `enabled` | Per-CWD configuration reload error; no Skills returned for that CWD |
| `enabled="false"` string | Per-CWD reload error: boolean required |
| Malformed TOML | Per-CWD configuration reload error |

The error cases still returned a JSON-RPC result envelope. The failure appears inside `result.data[].errors`, accompanied by `skills=[]`; consumers must not interpret it as a complete empty inventory. Missing `path` was accepted/ignored in this test, whereas missing `enabled` was rejected.

The fixture's symlink target is deliberately outside the native Skill root but inside the isolated fixture. Native support for it establishes matching behavior only. SkillRoster must continue requiring its own approved-root or explicit durable-read authority before following such a target.

### Effective-layer counterexample

The layer fixture creates an isolated Git repository, marks its canonical CWD trusted in the user config, and writes project `.codex/config.toml`. Both APIs run against that CWD:

| User config | Trusted project config | `config/read` merged array | `skills/list` user Skill |
| --- | --- | --- | --- |
| Native false, legacy false | Native true | Native true only | Native false; legacy false |
| Native true | Native false | Native false | Native true |

`config/read` with `includeLayers=true` confirmed the trusted project layer was active. In the untrusted control it included an explicit project-layer `disabledReason`, and the user configuration stayed effective. Thus the trusted result cannot be explained merely by the project being untrusted or undiscovered.

A CLI override `-c 'skills.config=[{path="/absolute/fixture/.../SKILL.md",enabled=true}]'` **did** enable a user Skill disabled by the user file in `skills/list`.

Observed conclusion: for these user Skills, the native list's enablement follows the user-file decision and CLI override, while differing from the generic merged project array returned by `config/read`. Do not assume `config/read.config.skills.config` reproduces Skill enablement, and do not infer that project settings never affect any Skill scope. The exact internal scope-selection mechanism, repository Skills, profiles, system/managed layers, and active-session catalog remain unverified.

### Supported observation scope

A user-file visibility adapter can deterministically observe both legacy and native user entrypoints with the following verified rules: read typed TOML entries in order; resolve entrypoint paths from the config directory with fixture-HOME tilde expansion; match existing canonical entrypoint identities; retain the last matching entry's boolean; never treat package-directory settings as an entrypoint disable. Preserve ignored missing-path behavior only if deliberately adopting this version's parser contract. Reject or mark unknown malformed configuration rather than counting its empty native result as reliable absence.

Name the evidence basis **Codex user-Skill local-file configuration** and record the exact CODEX_HOME/context used. It is not a universal effective-session resolver. CLI overrides can reverse user settings, so this projection cannot assert an arbitrary live conversation is disabled. If an exposure-dependent Plan depends on this scoped fact, the public contract must declare that scope, bind its configuration observations and identities, and block unsupported contexts rather than invent precedence.

## Minimum safe implementation contract

1. Preserve inventory independently of exposure. A verified disabled Codex placement remains searchable factual inventory, with a typed disabled visibility fact and `default_exposed=false`. Core planning must not claim it enabled merely because its directory exists. Unknown configuration remains possible exposure and cannot authorize an affected exposure decision.
2. Keep native settings read-only. Store only bounded settings observations, their digest or explicit absence, and the minimum relevant parsed decisions. Do not retain arbitrary config contents or credentials. Refuse special files and unintended settings symlinks; reuse the existing bounded safe-read approach where appropriate.
3. Make the observation scope explicit: local files and the supplied context, not the effective settings of an arbitrary running Codex conversation. Track every input needed for that scoped projection. Report/Find/Plan/Apply must reject stale or missing visibility evidence before reusing it to authorize a change.
4. Add `.agents/skills` discovery separately from configuration interpretation. Preserve legacy discovery and established write-target behavior. Reuse current approved-root containment, link trust, package fingerprint, and drift checks; native discovery permission does not endorse an external symlink target.
5. Avoid a new general configuration framework. Add only the bounded Codex-specific observation required for a verified contract, while keeping Claude's naming and existing Snapshot compatibility truthful.

## Remaining implementation boundaries

### Effective configuration and project trust

Codex uses TOML, path-addressed entries, and multiple configuration layers. The current Config basics documentation describes CLI overrides, trusted project layers, profiles, user settings, managed defaults, system settings, and built-ins. The native matrix confirms that untrusted project configuration is skipped, and that user-Skill enablement differs from generic trusted-project array merging. A user-config-only adapter must not claim to reproduce complete effective-session resolution.

Profiles, system/managed layers, repository Skills, nested project discovery, inaccessible files, and actual conversation behavior remain unverified. Resolve any of these before including it in exposure authority. Do not import Claude's JSON `skillOverrides` name-based merge rules into Codex.

If the first implementation intentionally observes only user configuration, name that basis and preserve unknown effective visibility when uninspected applicable layers can alter it. A verified local disabling observation can be reported as such without claiming effective runtime disablement. Decide the bounded scope before changing `default_exposed` or allowing Core Plans to rely on it.

### Path identity and source trust

The native matrix establishes entrypoint-only matching, config-directory-relative paths, tilde expansion, and canonical matching for existing symlinked entrypoints. Matching nonexistent targets, malformed paths, and identities changed during resolution remains unverified. Native canonicalization is not permission to resolve unapproved external sources.

Match verified existing entrypoints by canonical identity within the declared observation scope. Resolve paths only within already approved read boundaries; a config value naming a path is not a grant to read or canonicalize arbitrary external content. Keep the placement path, resolved source identity, and settings match identity distinct. Ambiguous, missing, escaped, or retargeted matches must not authorize a mutation. Freeze and recheck identities at existing Plan/Apply boundaries.

### Secondary shared root and reversible governance

`.agents/skills` is a shared conventional location; discovering it as Codex inventory does not prove Codex is its only consumer. Automatically treating it as an ordinary mutable Agent-exclusive root can hide a cross-Agent impact.

`src/roster_plan.rs::agent_root` currently picks the first included Skill root. Simply prepending another known root can redirect Core placement creation. Merely appending it also leaves an implicit destination choice when the legacy root is absent. Specify destination selection separately from observation order, preserving existing receipts and explicit-root behavior.

Minimum staged approach: observe the secondary root without expanding automatic mutation authority until shared-root governance is defined. If existing Mutation Scope values cannot honestly express read-only secondary discovery, make a narrow explicit schema decision rather than abusing provider/durable classifications. Later mutable support must account for every linked/shared consumer, preserve user contents, validate complete discovery across affected roots, record exact changed paths in the Receipt, and ensure Undo restores only those paths while refusing drift or unexpected occupants. Do not rewrite native settings to work around a disabled placement.

## Targeted acceptance before delivery

- Both user roots discovered; legacy roots and explicit roots retain their previous destination contract.
- Isolated native disabled fixture and SkillRoster visibility agree within the declared observation scope.
- Unknown/config-layer conflicts cannot authorize Core exposure; inventory remains available.
- Aliases, out-of-bound links, malformed configuration, duplicate entries, and settings drift fail safely without changing unrelated files.
- Affected Roster Apply and Undo fixtures cover shared-root consumers and exact Receipt ownership before secondary-root mutations are enabled.
- Repository checks required by AGENTS.md pass. A live isolated acceptance run identifies the exact Codex version; desktop behavior remains separately unverified until tested.

## Retained reproduction artifacts

- `/private/tmp/skillroster-codex-config-matrix.py`: base user-file, path, alias, duplicate, and malformed cases.
- `/private/tmp/skillroster-codex-config-layers.py`: initial project-layer controls.
- `/private/tmp/skillroster-codex-config-gitlayers.py`: explicit isolated Git project controls.
- `/private/tmp/skillroster-codex-config-readlayers.py`: records both `skills/list` and `config/read` layer evidence.
- `/private/tmp/skillroster-codex-config-overrides.py`: reverse project decision and CLI override.
- Base matrix results: `/private/tmp/skillroster-codex-matrix-oqmsk9kq/results.json`.
- Trusted/untrusted layer readback: `/private/tmp/skillroster-codex-matrix-jpr5j87p/`.
- Reverse and CLI override results: `/private/tmp/skillroster-codex-matrix-tiwhumxc/results.json`.

No version-pinned upstream source was inspected. The installed 0.163.0 executable and saved native responses are the exact-version primary evidence; current official documentation supplies broader product guidance.

# Codex native discovery and visibility gap

Inspected 2026-10-22. The initial gaps below describe installed SkillRoster 1.8.46
and the pre-adapter repository. Later sections record native controls and
working-tree comparisons. The implemented boundary and verification are in
[the acceptance record](../acceptance/codex-user-skill-visibility-2026-10-22.md).

## Evidence boundary

- Installed CLI: `codex-cli 0.163.0`, executable `/opt/homebrew/bin/codex`.
- Native test: that executable's local `app-server --stdio`, initialized over JSON-RPC, then `skills/list` with an isolated project CWD and `forceReload: true`. No model turn or account API was called.
- Initial comparison binary: installed `skillroster 1.8.46`, not a fresh working-tree build. The later selector and regression comparisons explicitly use the locally built candidate.
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
| Entry missing both `path` and `name`, false | Valid load; fixture Skills remain enabled |
| Entry missing `enabled` | Per-CWD configuration reload error; no Skills returned for that CWD |
| `enabled="false"` string | Per-CWD reload error: boolean required |
| Malformed TOML | Per-CWD configuration reload error |

The error cases still returned a JSON-RPC result envelope. The failure appears inside `result.data[].errors`, accompanied by `skills=[]`; consumers must not interpret it as a complete empty inventory. Missing both selectors was accepted/ignored in this test, whereas missing `enabled` was rejected. Missing only `path` does not establish an ignored entry: the follow-up matrix below confirms a supported `name` selector.

### Name selectors and ordered matching

A follow-up on 2026-10-22 used the same installed **Codex 0.163.0** and isolated no-model app-server protocol. Forty cases varied fixture manifests and user configuration; full native responses and error arrays were retained in the two result directories listed below. The frontmatter-name controls deliberately used a directory named `native-user` with `name: display-skill`.

| User `skills.config` input or fixture control | Native result |
| --- | --- |
| `name="display-skill"`, false, no path | The Skill with that frontmatter name disabled |
| `name="native-user"`, false, frontmatter name `display-skill` | Remains enabled; the directory name is not the selector |
| `name="native-user"`, false, same frontmatter name | Disabled |
| `name="NATIVE-USER"`, false | Remains enabled; case-sensitive matching |
| Name surrounded by spaces, tabs/newlines, or Unicode NBSP | Surrounding whitespace ignored; matching Skill disabled |
| `name="native user"` against `native-user`, or `name="*"` | Remains enabled; these do not match |
| Empty name, no path | Valid load; fixture Skills remain enabled |
| Same name twice, false then true / true then false | Enabled / disabled; last matching entry wins |
| Name-only false then matching path-only true | Enabled |
| Path-only false then matching name-only true | Enabled |
| Name-only true then matching path-only false | Disabled |
| Name-only false then unmatched name/path true | Remains disabled; unmatched entries do not reset the decision |
| Both name and exact matching entrypoint path, false | Remains enabled; this is not a conjunction or a path fallback |
| Both selectors, with name/path mismatch, empty or whitespace name, empty or nonexistent path | Valid load; fixture Skills remain enabled |
| Matching name-only false then both matching selectors true | Remains disabled; the both-selector entry is inert |
| Matching name-only false then neither selector true | Remains disabled; the selectorless entry is inert |
| Symlink-path false then matching name-only true | Linked Skill enabled |
| Name-only true then canonical source-path false | Linked Skill disabled |
| Canonical source-path false then both name and symlink path true | Remains disabled; the both-selector entry is inert |
| Identical frontmatter name in legacy and native user roots, name-only false | Both placements returned, both disabled |
| Non-string name (integer or array), with or without a valid path | Per-CWD reload error; no Skills returned for that CWD |
| Name-only entry missing `enabled` | Per-CWD reload error; no Skills returned for that CWD |

The bounded observed rule is: validate each relevant entry's field types, then apply entries in file order. Exactly one present selector participates: trim the name selector's surrounding whitespace and compare it case-sensitively with the frontmatter name, or match a path to the canonical entrypoint under the existing path rules. Both-present and neither-present entries do not participate, even when a selector is empty. Retain the boolean from the last participating entry matching that placement; default to enabled when none matches. Name matching can affect multiple placements and must not be collapsed into a one-name/one-placement map. The same ordered evaluation must cover a proposed Core source's verified frontmatter name and canonical entrypoint identity.

This is an observation of user Skills in 0.163.0, not a general selector schema for every Skill scope. The source has not been inspected to prove every possible Unicode whitespace code point; ASCII whitespace and NBSP controls support surrounding-whitespace trimming, while inner characters and case remain significant.

Two additional placement-name controls confirm that the matching name must stay bound to the inspected entrypoint, rather than a logical identity's representative metadata:

- Legacy and native manifests declared the same `source: fixture/shared` and `version: 1.0`, but retained distinct frontmatter names `legacy-user` and `native-user`. Native returned both placements and applied a name-only rule to only its matching name. Logical source/version grouping must not copy one placement's name onto another.
- Valid YAML `name: native-user # fixture comment` and `name: "native-user" # fixture comment` both produced native name `native-user`; a matching name-only false rule disabled them. A quoted name with surrounding whitespace also produced `native-user`. Metadata read permission alone does not establish native-equivalent parsing; unsupported name syntax must remain unknown instead of silently bypassing name rules.
- Valid YAML `name: native-user` followed by the indented continuation `  suffix` produced native name `native-user suffix`. A name-only false rule for that complete name disabled it. A bounded parser must reject unsupported scalar continuations as Unknown; skipping indented lines while trusting the initial scalar can incorrectly report On.

A local working-tree binary comparison reused all 40 selector fixtures, explicitly approving each fixture's `source/` directory for content inspection. The 35 valid-config fixtures agreed with native On/Off decisions for all named user placements; four malformed Skill-control fixtures remained Unknown, and the unrelated invalid-host-field control retained the declared local-rule projection with host validity unverified. These cases do not replace the placement-name controls above. Results and native responses remain separately inspectable in the artifact list.

A subsequent working-tree build with Snapshot visibility version `codex-user-skill-config-v3` repeated the 40-fixture comparison with the same outcome and correctly matched both source/version-name controls, both inline-comment controls, and the quoted-whitespace name control. The unsupported plain-scalar continuation above remains a separate regression boundary; passing the 40-fixture matrix alone does not establish it.

### Skill controls versus host configuration validity

A separate control set `model=4`, both alone and before a valid name-only disabling entry. Native 0.163.0 returned a per-CWD reload error (`invalid type: integer`, expected string) and no Skills. Thus successful TOML parsing and valid `skills.config` controls do **not** prove that the complete Codex configuration can load.

A bounded adapter need not duplicate Codex's unrelated host schema. It can truthfully expose `skill_configuration_known` for the parsed user-Skill controls, with `host_config_validity="unverified"` and `session_catalog_observed=false`. In that contract, On means no participating local user rule disables the inspected placement; Off means the last matching local user rule disables it. Neither state guarantees a running Codex catalog, successful configuration reload, or the result of an arbitrary invocation. Malformed TOML, unreadable settings, and malformed relevant Skill-control fields remain Unknown because they prevent this local rule projection itself.

Exposure counts and Core Plans may use that deliberately scoped local user-rule projection only if their public contract states this boundary and does not promise native runtime enablement. A request to establish an actually usable native catalog or running-session exposure additionally requires native validation in the exact invocation context. Renaming `configuration_known` without these state definitions and host-validity disclosure would leave the original ambiguity in place.

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

A user-file visibility adapter can deterministically observe both legacy and native user entrypoints with the following verified rules: read typed TOML entries in order; apply the exactly-one-selector rule above; match trimmed case-sensitive frontmatter names or resolve entrypoint paths from the config directory with fixture-HOME tilde expansion; match existing canonical entrypoint identities; retain the last matching entry's boolean; never treat package-directory settings as an entrypoint disable. Ignore selectorless and both-selector entries only when deliberately adopting this version's observed parser contract. Reject or mark Unknown malformed TOML or relevant Skill-control fields rather than counting an empty native result as reliable absence. Complete host-configuration validity remains a separate, unverified fact for a local-control-only adapter.

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

The native matrix establishes entrypoint-only matching, config-directory-relative paths, tilde expansion, and canonical matching for existing symlinked entrypoints. Matching nonexistent targets, malformed paths, and identities changed during resolution remains unverified. Canonicalization may observe metadata identity for configured paths, including external paths; it grants no permission to read external Skill contents.

Match verified existing entrypoints by canonical identity within the declared observation scope. Reading Skill contents and obtaining a verified frontmatter name still require the existing approved-root or durable-read boundary; a configured path does not add trust or mutation authority. Keep the placement path, resolved source identity, and settings match identity distinct. Ambiguous, missing, escaped, or retargeted matches must not authorize a mutation. Freeze and recheck identities at existing Plan/Apply boundaries.

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
- Name selector reproduction: `/private/tmp/skillroster-codex-name-selectors.py`; 28-case results and full per-case native responses: `/private/tmp/skillroster-codex-matrix-huo8jwh7/`.
- Additional 12-case name whitespace, both-selector, selectorless, and malformed controls: `/private/tmp/skillroster-codex-matrix-gaoni8nf/`.
- Host-config type-error control with `model=4` alone: `/private/tmp/skillroster-codex-matrix-b68tzivu/model-int/response.json`; the same host error before valid Skill controls is retained in the name-selector matrix's `host-model-invalid/response.json`.
- Working-tree binary comparison script: `/private/tmp/skillroster-codex-name-compare.py`; 40-case scoped comparison results: `/private/tmp/skillroster-codex-name-compare-1792672594233910000/results.json`.
- Distinct placement names under one declared source/version: `/private/tmp/skillroster-codex-matrix-8to9bouj/`; YAML inline-comment and quoted-whitespace name controls: `/private/tmp/skillroster-codex-matrix-kx60qnar/`. Each case retains its native `response.json` and a fixture-only `sr-state/` database from the working-tree comparison.
- Version-v3 40-fixture comparison: `/private/tmp/skillroster-codex-name-compare-1792672925162856000/results.json`; five placement-name regression comparisons: `/private/tmp/skillroster-codex-name-v3-regressions-1792672940897460000/results.json`.
- Native YAML plain-name continuation and version-v3 comparison: `/private/tmp/skillroster-codex-matrix-yw0k3gxw/name-plain-continuation/`.
- Final 40-case replay: `/private/tmp/skillroster-codex-name-compare-1792673099521366000/results.json`, with no valid-config mismatches. The continuation fix was replayed against the same native fixture and correctly retained Unknown; receipt: `/var/folders/25/mpff0p7j7slgl5fj961tl7480000gp/T/skillroster-codex-continuation-final-5kfjezv5/acceptance.json`.

No version-pinned upstream source was inspected. The installed 0.163.0 executable and saved native responses are the exact-version primary evidence; current official documentation supplies broader product guidance.

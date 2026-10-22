# Codex user-Skill discovery and visibility

Date: 2026-10-22. Base: `4f9fb2c`; published CLI version remains 1.8.46.

## Reproduced problems

An isolated Codex 0.163.0 app-server discovered user Skills in both
`.codex/skills` and `.agents/skills`. The installed SkillRoster 1.8.46 omitted
the latter by default. Explicitly including it discovered the files but still
reported a natively disabled Skill as exposed and mutable.

The native comparison used temporary HOME, CODEX_HOME, XDG_CONFIG_HOME and CWD,
JSON-RPC `initialize` and `skills/list`, and no model turn. The version-specific
mechanism evidence is in [the research record](../research/codex-native-visibility.md).

## Implemented contract

- Discover both user roots while preserving the legacy mutable destination
  order. `.agents/skills` and its readable physical aliases are shared read-only
  inventory, not Bootstrap/Core write destinations or proof of Codex installation.
- Observe the supplied Home's `.codex/config.toml` user-Skill rules: typed TOML,
  ordered name/path selectors, exactly one selector per participating rule,
  config-relative and Home-relative paths, and canonical entrypoint matching.
- Freeze names from each verified manifest before source/version identity
  grouping. Supported single-line names, basic quotes and inline comments match
  the native controls. Unsupported syntax, including multiline names, remains
  unknown when it can affect the result; it cannot silently bypass a name rule.
- Keep disabled Skills in inventory and available for explicit verified loading.
  Their local default exposure is false. Unknown policy retains possible exposure
  and cannot authorize a dependent change.
- Guard Core creation and implicit cross-Agent retargeting. Library consolidation
  includes the unchanged canonical placement and nested physical consumers;
  source updates check the replacement manifest name across physical consumers.
  These operations must preserve applicable native restrictions.
- Freeze configuration and root/path identities. An older Snapshot, changed
  configuration or retargeted root requires a new Scan before Report, Find,
  Plan, Apply or Setup, including `--check`.

Scan, Report and Plan disclose `skill_configuration_known`,
`host_config_validity: "unverified"` and `session_catalog_observed: false`.
On/Off are local user-rule projections. They do not validate Codex's complete
host configuration, CLI overrides, project/managed layers, or an active session.
Neither discovery nor governance rewrites user configuration.

## Independent review and replay

Review found and the implementation closed these concrete bypasses:

1. An old source-only Snapshot could omit the new authority observation.
2. Retargeting an empty user root after Scan/Ready Plan could redirect a write
   into shared inventory.
3. Demoting a Claude canonical source could retarget a disabled Codex link and
   change its native matching identity.
4. Consolidating two copies with opposite path rules could merge rule identities
   and enable the formerly disabled canonical copy. A nested child consumer also
   required coverage even when it had a different logical Skill identity.
5. Updating a shared manifest name could change a Codex consumer's name rule.
6. Representative metadata from same-source variants, inline comments and name
   continuations could supply an incorrect matching name.

The final binary replay of the 40 native selector fixtures had no mismatches in
the 35 valid-config cases. Four malformed relevant-config cases stayed Unknown;
the unrelated invalid host-field case retained the explicitly scoped rule result.
Distinct-name same-source fixtures, quoted names and inline comments also matched.
The native multiline-name control now returns Unknown rather than a false On.

Public CLI regressions exercise disabled Find/load, Core rejection, malformed
configuration, legacy Snapshots, config/alias/root drift, shared-root aliases,
Setup refusal, Library consolidation and source updates. Successful unrelated
Apply/Undo preserves configuration and shared/disabled Skill bytes. Name-only
and no-policy Library consolidation preserve their original exposure states.

## Validation

- `bash scripts/ci-full-check.sh`: passed; 534 Rust tests passed with one existing
  ignored test, 152 Node tests passed, strict Clippy, formatting, README evidence,
  installation surface and release archive checks passed.
- `cargo build --locked --offline` and `cargo run --locked --offline -- --help`:
  passed.
- Final human-output scope annotation: 24 presentation tests passed, a fresh
  build passed, and temporary Scan/Report output disclosed the local-rule scope.
  Formatting and all-target/all-feature strict Clippy were checked again.
- Independent governance review replayed the original Library bypass and the
  successful name-only/no-policy controls; no further issue was found in that
  reviewed scope. The name-continuation correction has its native-fixture replay
  and focused regression above.
- `git diff --check`: passed.

Two preexisting root-retarget tests now assert the earlier typed
`codex_visibility_root_identity_changed` rejection, retaining their no-write,
Receipt and Undo checks. The legacy-Ready-Plan test now uses a conventional Codex
user root so it reaches its intended legacy-Snapshot rejection instead of the
new unknown-policy blocker for arbitrary explicit roots.

## Delivery boundary

This is local implementation and fixture/native-CLI verification. It does not
establish merge, release, installed-binary upgrade, desktop-session acceptance,
or task-quality/token savings. Shared-root mutation remains unsupported.

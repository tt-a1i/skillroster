# Repository Guidelines

## Project Structure & Module Organization

`src/main.rs` contains the Rust CLI entry point. Move reusable logic into focused modules under `src/` and keep `main.rs` to parsing and orchestration. Place integration tests in `tests/`; keep small unit tests beside modules with `#[cfg(test)]`. Canonical vocabulary lives in `CONTEXT.md`; normative requirements live in `docs/product-spec.md`. Keep generated `target/` artifacts untracked.

## Build, Test, and Development Commands

- `cargo build` compiles a debug binary.
- `cargo run -- --help` runs the local CLI and verifies its public help text.
- `cargo test` runs all unit and integration tests.
- `cargo fmt --check` checks formatting without modifying files.
- `cargo clippy --all-targets --all-features -- -D warnings` treats lint warnings as failures.

Run these checks before requesting review.

## Coding Style & Naming Conventions

Use Rust 2024 and the repository's declared minimum toolchain. Let `rustfmt` control indentation and layout. Name modules, functions, variables, and test cases with `snake_case`; types and traits with `PascalCase`; constants with `SCREAMING_SNAKE_CASE`. Keep CLI nouns consistent with the product model: Library, Roster, On-demand Skill, Evidence, Finding, Plan, and Receipt. Prefer small deterministic components and structured output over prose that callers must scrape.

## Testing Guidelines

Write tests for core logic and high-risk boundaries: inventory normalization, roster selection, plan validation, filesystem mutation, and undo behavior. Routine CLI wiring, documentation, and low-risk mechanical changes do not require new tests. Add a regression test when fixing a core-logic bug. Name tests after observable behavior, for example `apply_refuses_a_drifted_plan`. Mutation tests must use temporary directories and verify the resulting state and undo receipt. No coverage percentage is required.

## Commit & Pull Request Guidelines

Write short commit messages in the form `type: summary`, matching the existing `chore: initialize SkillRoster`. Useful types include `feat`, `fix`, `docs`, `test`, and `refactor`; for example, `feat: add read-only skill scan`. Keep commits narrow and verifiable. When opening a pull request, explain the problem and solution, link the relevant issue, and list the checks run. Include representative CLI or JSON output for user-visible changes.

## Architecture & Safety

SkillRoster is a local-first engine for Agents. Keep semantic comparison, intent interpretation, prioritization, and explanation in the caller. The Rust CLI returns bounded deterministic facts, validates structured decisions, and executes reversible Plans; people need not compose command workflows. Preserve user-owned contents, refuse ambiguity, and produce a Receipt for every mutation. Read `CONTEXT.md` and `docs/product-spec.md` before changing this boundary, domain terms, or safety rules.

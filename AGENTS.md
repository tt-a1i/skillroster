# Repository Guidelines

## Project Structure & Module Organization

`src/main.rs` contains the Rust CLI entry point. As behavior grows, move reusable logic into focused modules under `src/` and keep `main.rs` limited to argument parsing and orchestration. Place integration tests in `tests/`; keep small unit tests beside their modules with `#[cfg(test)]`. Product boundaries and canonical vocabulary live in `docs/product-brief.md`. Cargo build artifacts under `target/` are generated and must remain untracked.

## Build, Test, and Development Commands

- `cargo build` compiles a debug binary.
- `cargo run -- --help` runs the local CLI and verifies its public help text.
- `cargo test` runs all unit and integration tests.
- `cargo fmt --check` checks formatting without modifying files.
- `cargo clippy --all-targets --all-features -- -D warnings` treats lint warnings as failures.

Run formatting, Clippy, and tests before requesting review.

## Coding Style & Naming Conventions

Use Rust 2024 and the repository's declared minimum toolchain. Let `rustfmt` control indentation and layout. Name modules, functions, variables, and test cases with `snake_case`; types and traits with `PascalCase`; constants with `SCREAMING_SNAKE_CASE`. Keep CLI nouns consistent with the product model: Library, Roster, Cold Skill, Evidence, Plan, and Receipt. Prefer small deterministic components and structured output over prose that callers must scrape.

## Testing Guidelines

Write tests for core logic and high-risk boundaries: inventory normalization, roster selection, plan validation, filesystem mutation, and undo behavior. Routine CLI wiring, documentation, and low-risk mechanical changes do not require new tests. Add a regression test when fixing a core-logic bug. Name tests after observable behavior, for example `apply_refuses_a_drifted_plan`. Mutation tests must use temporary directories and verify the resulting state and undo receipt. No coverage percentage is required.

## Commit & Pull Request Guidelines

Write short commit messages in the form `type: summary`, matching the existing `chore: initialize SkillRoster`. Useful types include `feat`, `fix`, `docs`, `test`, and `refactor`; for example, `feat: add read-only skill scan`. Keep commits narrow and independently verifiable. When opening a pull request, explain the problem and solution, link the relevant issue when one exists, and list the checks run. Include representative CLI or JSON output for user-visible changes.

## Architecture & Safety

SkillRoster is local-first and agent-first. V1 is a Rust CLI plus one thin bootstrap Skill, not an MCP server. Preserve user-owned Skill contents, preview mutations as immutable plans, refuse ambiguous targets, and produce receipts for every applied change. Read `docs/product-brief.md` before changing domain terms or safety boundaries.

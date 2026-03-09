---
name: criew-development
description: Repository-specific workflow and coding rules for the CRIEW codebase. Use when modifying or reviewing CRIEW Rust code, TUI behavior, sync/IMAP/reply/patch workflows, migrations, tests, docs, or config, and whenever the task must follow `docs/development/code-guildline.md` or `docs/development/code-guildline-cn.md`.
---

# Criew Development

## Overview

Follow the CRIEW repository's coding rules, architecture boundaries, and validation workflow.
Read the repository docs first, then make focused changes that preserve the current mail, patch, and reply behavior.

## Start With The Project Docs

Read the relevant repository docs before editing code.

- Read `docs/development/code-guildline.md` first for the canonical coding rules.
- Read `docs/development/code-guildline-cn.md` when the user works in Chinese or explicitly asks for the Chinese guideline.
- Read `README.md` or `README-zh.md` before changing user-visible behavior, install steps, naming, or operator workflow.
- Read `docs/architecture/design.md` before changing architecture, module boundaries, sync flow, or data-model assumptions.
- Read `docs/specs/reply-format-spec.md` before changing reply composition, quoting, headers, or send flow.
- Read `docs/reference/config.example.toml` before changing config keys, defaults, or path semantics.

Treat `docs/development/code-guildline.md` as the priority source when a local convention is unclear.
Then follow tool-enforced rules and the existing style in the touched module.

## Keep The CRIEW Boundaries

Use the existing layer split unless the task explicitly asks for architectural rework.

- Keep `src/app/` focused on use-case orchestration and CLI/TUI entry workflows.
- Keep `src/domain/` focused on core models and business meaning.
- Keep `src/infra/` focused on storage, config, IMAP, sendmail, `b4`, logging, and other external integrations.
- Keep `src/ui/` focused on TUI state, rendering, input handling, and UI tests.
- Keep database changes in `migrations/` and align schema changes with the Rust storage code in the same change.
- Treat `vendor/b4/` as vendored third-party code; avoid editing it unless the task explicitly targets that dependency.

Preserve the current naming set from the README:
use `CRIEW` for the repository name and `criew` for the crate, CLI, config file, runtime directory, and environment variables.
Do not reintroduce legacy Courier naming.

## Apply The Coding Rules Directly

Implement changes in the style required by `docs/development/code-guildline.md`.

- Prefer descriptive, behavior-accurate names.
- Encode units in names when the type system does not.
- Use assertion-style names for booleans.
- Keep one primary concept per file; split large files early.
- Keep code readable from top to bottom, with high-level flow before helper detail.
- Keep functions small and focused; reduce nesting with early returns.
- Avoid ambiguous boolean arguments; prefer enums or small config structs.
- Express invariants in types when practical.
- Propagate fallible paths with `?`.
- Keep visibility and lint suppression as narrow as possible.
- Add `// SAFETY:` comments for every `unsafe` block and document `# Safety` on unsafe APIs.
- Write comments only when they explain why, constraints, or design tradeoffs.

## Use A Repository-Focused Workflow

Follow this order unless the task is trivial.

1. Locate the relevant module with `rg` and read the surrounding code before proposing changes.
2. Confirm the user-visible behavior from the README or a spec doc when the change affects sync, reply, patch apply/export, config, or startup flow.
3. Keep the change focused on one logical topic.
4. Add or update regression tests when fixing a bug or changing observable behavior.
5. Update docs when behavior, commands, config, or workflow changed.

Prefer updating both `README.md` and `README-zh.md` when the change affects user-facing usage or setup.
Keep PR-sized changes focused and commit subjects aligned with the repository's Conventional Commit prefixes:
`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`.
When asked to create a commit in this repository, use `git commit -s`.
The repository hook at `.githooks/commit-msg` and CI both validate the `Signed-off-by:` trailer.
Simple commits may use only the subject line.
For larger commits, add a body with bullet points that summarize the main changes.

## Run The Required Validation

Run the full repository validation set after non-trivial changes when the environment allows it.

- Keep overall repository coverage at or above 70%.
- Keep newly added code at or above 80% coverage.
- Keep critical-component coverage at or above 85% for critical workflow code that the current change directly touches or materially expands. Treat sync, reply, patch, and other core user-facing workflow paths in scope as critical unless the task clearly falls outside those paths.
- Do not add tests only to tick uncovered lines. Coverage work must defend a real behavior, regression, failure mode, or workflow contract that matters to users or operators.
- Prefer behavior-driven regression tests over line-chasing. If a remaining gap is not worth a brittle or artificial test, leave it uncovered and report the tradeoff clearly.
- Use `./scripts/check-coverage.sh` plus the generated summary report to verify the thresholds above, and review file-level or workflow-level reports for the in-scope critical components. Treat threshold regressions as incomplete work.

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets --all-features`
- `./scripts/check-coverage.sh`

If time or environment constraints prevent a command from running, report that clearly and avoid claiming full verification.

## Pull The Right Context On Demand

Load extra repository docs only when the task needs them.

- Read `docs/milestones/mvp-milestones.md` and `docs/milestones/reply-mvp-milestones.md` for historical intent or rollout sequencing.
- Read `docs/milestones/vim-mvp-milestones.md` and `docs/specs/code-preview-vim-prototype.md` for Vim-mode and code-preview behavior.
- Read `src/ui/tui/tests.rs` before extending TUI behavior that already has test coverage.

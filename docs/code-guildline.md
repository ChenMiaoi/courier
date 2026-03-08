# CRIEW Coding Guidelines

## Scope And Priority

Priority order for style and review decisions:
1. This document (`docs/code-guildline.md`)
2. Tool-enforced rules (`rustfmt`, `clippy`)
3. Existing local conventions in the modified module

Rule levels used in this document:
- Must: every new or modified code path must satisfy the rule.
- Recommended: satisfy by default unless there is a clear tradeoff.
- Conditional: applies only when the corresponding code shape exists, such as `unsafe`.

## General Rules

Must:
- `descriptive-names`: names should be self-explanatory at the usage site.
- `accurate-names`: names must reflect behavior and side effects precisely.
- `encode-units`: encode units in names when the type system cannot express them, such as `*_bytes` or `*_ms`.
- `bool-names`: boolean names should read like assertions, such as `is_*`, `has_*`, `can_*`, or `should_*`.
- `explain-why`: comments should explain why, not restate what the code already says.
- `design-decisions`: record non-obvious design decisions and rejected alternatives.
- `one-concept-per-file`: keep one primary concept per file; split large files early.
- `top-down-reading`: keep code readable from top to bottom, with higher-level flow before details.
- `logical-paragraphs`: organize function bodies into logical paragraphs.
- `error-message-format`: keep error messages specific and stylistically consistent.

Recommended:
- `semantic-line-breaks`: prefer semantic line breaks in Markdown and doc comments.
- `cite-sources`: cite sources when implementing external specifications or algorithms.
- `familiar-conventions`: prefer conventions familiar to Rust and Linux developers.

## Rust Rules

Must:
- `camel-case-acronyms`: types, traits, and acronyms should follow Rust naming conventions.
- `minimize-nesting`: prefer early returns to reduce nesting depth.
- `small-functions`: keep each function focused on a single responsibility.
- `no-bool-args`: avoid ambiguous boolean parameters; prefer enums or config structs.
- `rust-type-invariants`: express invariants in the type system whenever practical.
- `propagate-errors`: prefer `?` for fallible paths instead of manual branching.
- `narrow-visibility`: use the narrowest visibility possible, preferably private or `pub(crate)`.
- `narrow-lint-suppression`: keep lint suppression scoped as tightly as possible.
- `debug-assert`: use `debug_assert!` only for correctness checks that can be omitted in release builds.

Conditional:
- `justify-unsafe-use`: every `unsafe` block must include a `// SAFETY:` explanation.
- `document-safety-conds`: `unsafe fn` and unsafe preconditions must be documented under `# Safety`.
- `module-boundary-safety`: explain safety at the module boundary, not only at individual call sites.

Recommended:
- `explain-variables`: split complex expressions into semantically named intermediate variables.
- `block-expressions`: use block expressions to limit the lifetime of temporary state.
- `checked-arithmetic`: prefer checked or saturating arithmetic when overflow is possible.
- `enum-over-dyn`: prefer `enum` over trait objects for closed sets.
- `getter-encapsulation`: prefer encapsulation over exposing mutable internal state.
- `module-docs`: provide module-level documentation for major modules.
- `macros-as-last-resort`: treat macros as a last resort; prefer functions and traits first.
- `minimize-copies`: avoid unnecessary copies and allocations on hot paths.
- `no-premature-optimization`: optimize only after you have evidence, such as profiling or benchmarks.

## Testing Rules

Must:
- `add-regression-tests`: add regression tests when fixing bugs whenever practical.
- `test-visible-behavior`: test user-visible behavior instead of coupling tests to implementation details.
- `use-assertions`: use assertion macros instead of manual print-and-compare flows.
- `test-cleanup`: clean up files, directories, and child processes created by tests.
- `coverage-check`: modified code must keep the project coverage command runnable.

Project notes:
- Consistency tests should use user-visible disassembly output as the source of truth.
- Decoder fixes should usually add both Rust unit tests and matching consistency cases when applicable.
- Required validation commands before merge:
  `cargo fmt --all -- --check`
  `cargo clippy --all-targets --all-features -- -D warnings`
  `cargo test --all-targets --all-features`
  `./scripts/check-coverage.sh`

## Git And Pull Request Rules

Must:
- `atomic-commits`: keep one logical change per commit.
- `refactor-then-feature`: separate refactors from feature changes.
- `focused-prs`: keep each pull request focused on a single theme.
- `signed-commits`: create repository commits with `git commit -s` so each commit carries a valid `Signed-off-by:` trailer.
- `large-commit-body`: simple commits may use only the subject line, but larger commits must include a body with bullet points that summarize the main changes.

Commit message policy
(compatible with this project and the referenced Asterinas rules):
- Keep the project's Conventional Commit prefixes: `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`.
- An optional scope is allowed, such as `feat(ci): ...` or `fix(sync): ...`.
- Write the subject line in the imperative mood.
- Keep the subject line under 72 characters when practical.
- Use `git commit -s` for authored commits; `.githooks/commit-msg` and CI validate the `Signed-off-by:` trailer.
- Simple commits may use only the subject line.
- Larger commits must include a body with bullet points such as `- add ...` and `- refactor ...`.
- The current hook and CI treat a commit as larger when it touches at least 6 files or changes at least 150 added/deleted lines.

## Review Checklist

- [ ] Names are clear and accurate (`descriptive-names`, `accurate-names`).
- [ ] Units and boolean semantics are explicit (`encode-units`, `bool-names`).
- [ ] Comments explain motivation, and key design decisions are recorded (`explain-why`, `design-decisions`).
- [ ] Public interfaces do not leak implementation details (`hide-impl-details`).
- [ ] Functions stay focused, and nesting is controlled (`small-functions`, `minimize-nesting`).
- [ ] Error handling follows the `Result` and `?` style (`propagate-errors`).
- [ ] If `unsafe` is used, the safety explanation is complete (`justify-unsafe-use`, `document-safety-conds`).
- [ ] Bug fixes include regression tests (`add-regression-tests`).
- [ ] Tests validate observable behavior and clean up resources (`test-visible-behavior`, `test-cleanup`).
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets --all-features`, and `./scripts/check-coverage.sh` pass.
- [ ] Commits and pull requests stay atomic and focused (`atomic-commits`, `focused-prs`).
- [ ] Each authored commit carries a valid `Signed-off-by:` trailer (`signed-commits`).
- [ ] Larger commits include a bullet-point body that explains the main changes (`large-commit-body`).

## Gradual Adoption

These guidelines apply to new and modified code.
Legacy code may not satisfy every rule yet.
When touching older code,
prefer small, low-risk, verifiable cleanups
that move the codebase toward these standards over time.

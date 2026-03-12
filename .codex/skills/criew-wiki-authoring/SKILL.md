---
name: criew-wiki-authoring
description: Write and revise pages under `docs/wiki` for the CRIEW GitHub wiki and its MkDocs-backed GitHub Pages site. Use when Codex needs to create, expand, reorganize, or review CRIEW wiki pages such as `Home.md`, `_Sidebar.md`, `_Footer.md`, or topic pages, when code or workflow changes require a wiki sync pass, and when the output or workflow must follow GitHub wiki conventions, the local MkDocs pipeline, and a pragmatic, kernel-documentation writing style.
---

# Criew Wiki Authoring

## Overview

Write CRIEW wiki pages as maintainer documentation.
Treat `docs/wiki/` as the source GitHub wiki repository, and treat the published website as a derived MkDocs build driven from the main `CRIEW` repository.
Keep the source compatible with both GitHub wiki rendering and the local Pages build pipeline, and prefer a direct, technical style over narrative or marketing copy.
Use this skill not only for standalone wiki work, but also when a code or config change means the existing wiki may now be stale.

## Follow This Workflow

1. Confirm the source of truth before writing.
- Read the code, README, spec, config example, or architecture note that defines the behavior.
- Prefer `README.md`, `README-zh.md`, `docs/architecture/design.md`, `docs/specs/`, and `docs/reference/config.example.toml` depending on the topic.
- Do not invent commands, defaults, limitations, or workflows.
- When paired with a code change, identify which existing wiki pages describe the changed behavior and either update them or explicitly confirm that no wiki page is affected.

2. Check the wiki context.
- List the existing pages in `docs/wiki/` before creating or renaming a page.
- Treat `docs/wiki/` as a separate Git repository. Use `git -C docs/wiki ...` when checking history or status.
- Read `references/publish-workflow.md` before changing local preview, staging, or deployment behavior.
- Keep `Home.md` as the landing page and update it when a new top-level page changes how readers enter the wiki.
- Read `references/style-guide.md` for page rules.
- Read `references/page-patterns.md` when choosing a page shape.

3. Choose the smallest page that fits the job.
- Use a short reference page for stable facts and concepts.
- Use a workflow page for operator tasks with prerequisites and ordered steps.
- Use a troubleshooting page for symptoms, likely causes, and recovery actions.
- Split a page once it starts carrying more than one primary purpose.

4. Draft in GitHub wiki form.
- Use page names and filenames that are stable, literal, and easy to scan.
- Prefer normal Markdown links such as `[Configuration](Configuration.md)` because they work in both GitHub wiki and MkDocs.
- Treat `[[Page Name]]` and `[[Page Name|Link text]]` as compatibility syntax only. The local staging script rewrites them for MkDocs, but new content should not depend on that rewrite when a Markdown link is clear enough.
- Use full GitHub URLs when linking from the wiki back into the main CRIEW repository, because the wiki is a separate repository.
- Add `_Sidebar.md` or `_Footer.md` only when shared navigation or repeated context is materially useful.

5. Check the publish path before finishing.
- Local copy lint goes through `./scripts/wiki-lint.sh`.
- Local preview and local build go through `./scripts/wiki-site.sh serve` and `./scripts/wiki-site.sh build`.
- The staging step copies `docs/wiki/` into `target/wiki-docs/`, turns `Home.md` into the MkDocs `index.md`, excludes special wiki-only files such as `_Sidebar.md` and `_Footer.md`, and normalizes source-only links.
- The published website is built from `mkdocs.yml` and deployed by `.github/workflows/wiki-pages.yml`.
- The lint script requires `autocorrect`. If it is missing, the script downloads a local copy into `target/wiki-venv/bin/` and then reruns the check.
- Treat a clean `./scripts/wiki-lint.sh` result as part of done for every wiki page this skill creates or edits.
- Because `docs/wiki` is a submodule, the main repository deploys the pinned wiki commit, not the remote wiki repository's latest HEAD. A new wiki commit reaches GitHub Pages only after the CRIEW repository updates the `docs/wiki` submodule pointer.
- If you commit inside `docs/wiki`, explicitly ask the user whether the main `CRIEW` repository should also commit the updated `docs/wiki` submodule pointer. Do not assume they want both commits automatically.

6. Draft in kernel-documentation style.
- Lead with scope and purpose.
- Prefer active voice, short sentences, and concrete nouns.
- State prerequisites, constraints, and side effects before optional detail.
- Prefer commands, paths, config keys, and observable outcomes over abstract explanation.
- Avoid filler, advocacy, roadmap prose, and vague adjectives such as `simple`, `easy`, `powerful`, or `seamless`.
- Name conditions directly when behavior depends on mode, configuration, or state.

7. Review before finishing.
- Verify commands, file paths, environment variables, config keys, and feature names against the repository.
- Keep language consistent within the page. Match the page's existing language unless the user requests a change.
- Check that headings are informative, examples are minimal, and links resolve to the intended wiki page or repository URL.
- Run `./scripts/wiki-lint.sh` for every text change and revise the page until it passes. If the environment prevents the check, report that explicitly instead of assuming the page is clean.
- When page structure or links changed, run `./scripts/wiki-site.sh build` if the environment allows it.

## Apply These Page Rules

- Start each page with a short paragraph that states what the page covers and when to use it.
- Prefer headings such as `Prerequisites`, `Workflow`, `Configuration`, `Troubleshooting`, `Limitations`, and `See also` when they fit the content.
- Keep heading depth shallow unless the page genuinely needs more structure.
- Keep lists parallel and action-oriented.
- Use fenced code blocks for commands, config fragments, and sample output.
- Keep examples minimal but real.
- Write copy that passes `autocorrect`; treat lint failures as defects that need wording changes before the page is considered complete.
- Preserve CRIEW naming: use `CRIEW` for the project and `criew` for the binary, crate, config file, runtime directory, and environment variables.

## Use The References

- `references/style-guide.md`: GitHub wiki constraints, linking rules, and writing expectations.
- `references/page-patterns.md`: Reusable page skeletons for `Home.md`, workflow pages, reference pages, and troubleshooting pages.
- `references/publish-workflow.md`: The `docs/wiki -> MkDocs -> GitHub Pages` pipeline, local preview commands, and submodule publication constraints.

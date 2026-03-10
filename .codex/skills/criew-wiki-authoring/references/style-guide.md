# CRIEW Wiki Style Guide

## GitHub Wiki Constraints

- Treat `docs/wiki/` as the source wiki repository, not as a normal docs folder inside the main repo.
- Treat the published website as a derived MkDocs build from the main `CRIEW` repository.
- Keep `Home.md` as the landing page.
- Use `_Sidebar.md` for shared navigation only when the page set is large enough to justify it.
- Use `_Footer.md` only for short, repeated context such as related links or maintenance notes.
- Prefer Markdown pages unless the existing page already uses another supported markup.
- Avoid syntax that GitHub wikis do not support well, such as definition lists, table-of-contents directives, transclusion, or heavy indentation tricks.

## Page Naming

- Use stable, descriptive page names.
- Keep filenames portable. Avoid characters that are awkward in URLs or file systems.
- Match the filename to the page topic instead of using vague names such as `Notes` or `Misc`.
- Rename pages only when the old name is clearly wrong or blocks navigation.

## Linking Rules

- Prefer standard Markdown links for internal page links, for example `[Configuration](Configuration.md)`.
- Keep `[[Page Name]]` or `[[Page Name|Link text]]` only as compatibility syntax for older pages. `scripts/prepare-wiki-site.py` rewrites them during MkDocs staging.
- Use normal Markdown links for external URLs.
- Use full GitHub URLs when linking from the wiki to files, directories, issues, or pull requests in the main `CRIEW` repository.
- Do not rely on repository-relative paths from the wiki back to the main repo. The wiki is a separate repository, so those links are easy to break.
- Keep link text explicit. Prefer the destination's role over generic text such as `here` or `more`.

## Writing Style

- State what the page is for in the opening paragraph.
- Prefer direct statements over explanation-first prose.
- Put prerequisites before steps.
- Put limitations and side effects near the action that triggers them.
- Prefer one fact per sentence when the topic is operational.
- Use active voice and imperative steps for procedures.
- Avoid marketing tone, vision statements, and rhetorical fillers.
- Avoid unexplained claims such as `fast`, `simple`, `robust`, or `advanced`.
- Keep wording compatible with `autocorrect`. If lint flags a phrase, rewrite the sentence instead of suppressing the check.

## Evidence And Accuracy

- Verify commands, config keys, and paths against the current repository state.
- Cite the real source in prose when the behavior comes from a spec, config example, or code path.
- Call out assumptions explicitly if the repository does not prove them.
- Prefer a small verified page over a broad speculative page.
- Run `./scripts/wiki-lint.sh` after every copy edit and treat a clean result as required for completion. If the environment prevents the check, report that gap explicitly.

## Page Maintenance

- Update `Home.md` when a new page changes the top-level information architecture.
- Merge or split pages when navigation starts to hide the main task flow.
- Keep the lint path working: `./scripts/wiki-lint.sh` installs `autocorrect` on demand when the binary is missing.
- Run `./scripts/wiki-site.sh build` after link or structure changes when the environment allows it.
- Remove stale TODO text before finishing.

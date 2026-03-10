# CRIEW Wiki Publish Workflow

## Pipeline

The publication path is:

`docs/wiki` source repo -> `scripts/prepare-wiki-site.py` staging -> `mkdocs.yml` build -> `.github/workflows/wiki-pages.yml` deploy -> GitHub Pages

This means the source of truth stays in the GitHub wiki repository, while the published website is generated from the main `CRIEW` repository.

## Local Commands

- `./scripts/wiki-lint.sh`: Check wiki copy with `autocorrect`. If the command is missing, the script downloads a local copy into `target/wiki-venv/bin/`.
- `./scripts/wiki-site.sh prepare`: Stage the wiki into `target/wiki-docs` without installing MkDocs.
- `./scripts/wiki-site.sh serve`: Stage the wiki, install MkDocs into `target/wiki-venv`, and start a local preview server on `0.0.0.0:8000` by default. Override the bind address with `CRIEW_WIKI_DEV_ADDR`.
- `./scripts/wiki-site.sh build`: Stage the wiki, install MkDocs into `target/wiki-venv`, and build `target/wiki-site`.

Treat `./scripts/wiki-lint.sh` as mandatory for wiki content changes.
Do not consider a new or edited wiki page complete until the copy passes the lint check, unless the environment prevents running the command and that limitation is reported.

## What The Staging Step Does

- Copy published wiki content from `docs/wiki/` into `target/wiki-docs/`.
- Rewrite `Home.md` into the MkDocs landing page `index.md`.
- Skip GitHub wiki helper files such as `_Sidebar.md` and `_Footer.md`.
- Rewrite legacy `[[Page]]` wiki links into Markdown links that MkDocs can resolve.

## CI And Deployment

- `.github/workflows/wiki-pages.yml` runs `./scripts/wiki-lint.sh` before it builds the site on pull requests that touch the wiki publish pipeline.
- The same workflow deploys to GitHub Pages on pushes to `develop`.
- The workflow uploads the rendered site from `target/wiki-site`.
- This CI step is the enforcement point for the same copy rules that the skill expects locally. Write pages to pass the lint check before handing the change off.

## Submodule Constraint

`docs/wiki` is a Git submodule in the main repository.
That means the GitHub Pages deployment uses the wiki commit pinned by the main `CRIEW` repository.
If the standalone wiki repository advances but the main repository does not update the `docs/wiki` gitlink, GitHub Pages will keep publishing the older pinned wiki snapshot.

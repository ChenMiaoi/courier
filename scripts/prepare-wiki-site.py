#!/usr/bin/env python3

from __future__ import annotations

import argparse
import posixpath
import re
import shutil
import sys
from pathlib import Path, PurePosixPath

MARKDOWN_SUFFIXES = {".md", ".markdown"}
SPECIAL_WIKI_FILES = {".git", "_Sidebar.md", "_Footer.md"}
WIKI_LINK_RE = re.compile(r"\[\[([^\]|]+?)(?:\|([^\]]+))?\]\]")
MARKDOWN_LINK_RE = re.compile(r"(?<!!)\[([^\]]+)\]\(([^)]+)\)")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Stage docs/wiki for MkDocs and normalize wiki-only links."
    )
    parser.add_argument(
        "--source",
        default="docs/wiki",
        help="GitHub wiki source directory.",
    )
    parser.add_argument(
        "--output",
        default="target/wiki-docs",
        help="Prepared MkDocs docs directory.",
    )
    return parser.parse_args()


def normalize_page_key(name: str) -> str:
    page_name = name.strip().removesuffix(".md").removesuffix(".markdown")
    page_name = page_name.split("#", 1)[0]
    page_name = page_name.rstrip("/")
    return re.sub(r"[\s_-]+", " ", page_name).strip().lower()


def is_markdown(path: Path) -> bool:
    return path.suffix.lower() in MARKDOWN_SUFFIXES


def should_skip(rel_path: Path) -> bool:
    return rel_path.name in SPECIAL_WIKI_FILES


def output_rel_for(source_rel: Path) -> Path:
    if source_rel == Path("Home.md"):
        return Path("index.md")
    return source_rel


def build_page_lookup(source_dir: Path) -> tuple[dict[str, PurePosixPath], dict[str, PurePosixPath]]:
    page_lookup: dict[str, PurePosixPath] = {}
    source_to_output: dict[str, PurePosixPath] = {}

    for source_path in sorted(source_dir.rglob("*")):
        if not source_path.is_file():
            continue

        source_rel = source_path.relative_to(source_dir)
        if should_skip(source_rel) or not is_markdown(source_rel):
            continue

        output_rel = PurePosixPath(output_rel_for(source_rel).as_posix())
        source_key = source_rel.as_posix()
        source_to_output[source_key] = output_rel

        page_key = normalize_page_key(source_rel.stem)
        existing = page_lookup.get(page_key)
        if existing is not None and existing != output_rel:
            raise ValueError(
                f"duplicate wiki page key '{page_key}' for {existing} and {output_rel}"
            )
        page_lookup[page_key] = output_rel

    home_page = page_lookup.get("home")
    if home_page is not None:
        page_lookup.setdefault("index", home_page)

    return page_lookup, source_to_output


def rel_link_to(output_target: PurePosixPath, current_output: PurePosixPath) -> str:
    current_parent = current_output.parent
    relative = posixpath.relpath(output_target.as_posix(), current_parent.as_posix() or ".")
    return relative


def rewrite_wiki_links(
    text: str,
    source_rel: Path,
    current_output: PurePosixPath,
    page_lookup: dict[str, PurePosixPath],
) -> str:
    errors: list[str] = []

    def replace(match: re.Match[str]) -> str:
        target_name = match.group(1).strip()
        link_text = match.group(2).strip() if match.group(2) else target_name
        target_output = page_lookup.get(normalize_page_key(target_name))
        if target_output is None:
            errors.append(
                f"{source_rel.as_posix()}: unresolved wiki link [[{target_name}]]"
            )
            return match.group(0)

        return f"[{link_text}]({rel_link_to(target_output, current_output)})"

    rewritten = WIKI_LINK_RE.sub(replace, text)
    if errors:
        raise ValueError("\n".join(errors))
    return rewritten


def normalize_local_markdown_target(
    target: str,
    source_rel: Path,
    current_output: PurePosixPath,
    source_to_output: dict[str, PurePosixPath],
) -> str | None:
    if not target or "://" in target or target.startswith("mailto:"):
        return None

    if target.startswith("#") or target.startswith("/"):
        return None

    fragment = ""
    target_body = target
    if "#" in target:
        target_body, fragment = target.split("#", 1)
        fragment = f"#{fragment}"

    target_body = target_body.strip()
    if not target_body:
        return None

    current_source = PurePosixPath(source_rel.as_posix())
    candidate_paths = [PurePosixPath(target_body)]
    if not PurePosixPath(target_body).suffix:
        candidate_paths.append(PurePosixPath(f"{target_body}.md"))

    for candidate in candidate_paths:
        joined = current_source.parent.joinpath(candidate)
        normalized = PurePosixPath(posixpath.normpath(joined.as_posix()))
        output_target = source_to_output.get(normalized.as_posix())
        if output_target is None:
            continue
        return f"{rel_link_to(output_target, current_output)}{fragment}"

    if target_body == "Home.md":
        return f"{rel_link_to(PurePosixPath('index.md'), current_output)}{fragment}"

    return None


def rewrite_markdown_links(
    text: str,
    source_rel: Path,
    current_output: PurePosixPath,
    source_to_output: dict[str, PurePosixPath],
) -> str:
    def replace(match: re.Match[str]) -> str:
        label = match.group(1)
        target = match.group(2)
        normalized = normalize_local_markdown_target(
            target,
            source_rel,
            current_output,
            source_to_output,
        )
        if normalized is None:
            return match.group(0)
        return f"[{label}]({normalized})"

    return MARKDOWN_LINK_RE.sub(replace, text)


def stage_wiki(source_dir: Path, output_dir: Path) -> None:
    if not source_dir.is_dir():
        raise FileNotFoundError(f"wiki source directory not found: {source_dir}")

    if not (source_dir / "Home.md").is_file():
        raise FileNotFoundError(
            f"expected {source_dir / 'Home.md'} so MkDocs can build an index page"
        )

    if output_dir.exists():
        shutil.rmtree(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    page_lookup, source_to_output = build_page_lookup(source_dir)

    for source_path in sorted(source_dir.rglob("*")):
        if not source_path.is_file():
            continue

        source_rel = source_path.relative_to(source_dir)
        if should_skip(source_rel):
            continue

        output_rel = output_rel_for(source_rel)
        destination = output_dir / output_rel
        destination.parent.mkdir(parents=True, exist_ok=True)

        if not is_markdown(source_rel):
            shutil.copy2(source_path, destination)
            continue

        text = source_path.read_text(encoding="utf-8")
        current_output = PurePosixPath(output_rel.as_posix())
        text = rewrite_wiki_links(text, source_rel, current_output, page_lookup)
        text = rewrite_markdown_links(text, source_rel, current_output, source_to_output)
        destination.write_text(text, encoding="utf-8")


def main() -> int:
    args = parse_args()
    source_dir = Path(args.source)
    output_dir = Path(args.output)

    try:
        stage_wiki(source_dir, output_dir)
    except Exception as exc:  # pragma: no cover - CLI error path
        print(exc, file=sys.stderr)
        return 1

    print(f"prepared MkDocs source at {output_dir} from {source_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

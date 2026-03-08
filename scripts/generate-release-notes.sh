#!/usr/bin/env bash

set -euo pipefail

tag_name=${1:?missing tag name}
output_file=${2:?missing output file}

repo_root="$(git rev-parse --show-toplevel)"
cd "${repo_root}"

package_name="$(
    python3 - <<'PY'
import tomllib

with open("Cargo.toml", "rb") as handle:
    cargo = tomllib.load(handle)

print(cargo["package"]["name"])
PY
)"

package_version="$(
    python3 - <<'PY'
import tomllib

with open("Cargo.toml", "rb") as handle:
    cargo = tomllib.load(handle)

print(cargo["package"]["version"])
PY
)"

tag_version="${tag_name#v}"
if [[ "${tag_version}" != "${package_version}" ]]; then
    printf '%s\n' \
        "tag ${tag_name} does not match Cargo.toml version ${package_version}" \
        >&2
    exit 1
fi

previous_release_tag="$(
    git tag --merged "${tag_name}" --sort=-version:refname 'v*' \
        | grep -Fxv "${tag_name}" \
        | head -n 1 \
        || true
)"

if [[ -n "${previous_release_tag}" ]]; then
    commit_range="${previous_release_tag}..${tag_name}"
else
    commit_range="${tag_name}"
fi

mkdir -p "$(dirname "${output_file}")"

{
    printf '## %s %s\n\n' "${package_name}" "${tag_name}"
    printf '[![codecov](https://codecov.io/github/ChenMiaoi/CRIEW/graph/badge.svg?token=AH99YLKKPD)](https://codecov.io/github/ChenMiaoi/CRIEW)\n\n'
    printf 'Source release for `%s`.\n\n' "$(git rev-list -n 1 "${tag_name}")"
    printf '### Release Assets\n\n'
    printf -- '- Source archive: `%s-%s-src.tar.gz`\n' "${package_name}" "${tag_name}"
    printf -- '- Source archive: `%s-%s-src.zip`\n\n' "${package_name}" "${tag_name}"

    if [[ -n "${previous_release_tag}" ]]; then
        printf '### What Changed Since `%s`\n\n' "${previous_release_tag}"
    else
        printf '### What Is Included In This Release\n\n'
    fi

    subjects_found=0
    while IFS= read -r subject; do
        [[ -n "${subject}" ]] || continue
        subjects_found=1
        printf -- '- %s\n' "${subject}"
    done < <(git log --reverse --format='%s' "${commit_range}")

    if [[ "${subjects_found}" -eq 0 ]]; then
        printf -- '- No commit subjects were found for this release range.\n'
    fi
    printf '\n'
} > "${output_file}"

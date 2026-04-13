#!/usr/bin/env bash

set -euo pipefail

tag_name=${1:?missing tag name}
output_file=${2:?missing output file}
asset_dir=${3:-$(dirname "${output_file}")}

resolve_python() {
    if command -v python3 >/dev/null 2>&1; then
        printf '%s\n' "python3"
        return
    fi

    if command -v python >/dev/null 2>&1; then
        printf '%s\n' "python"
        return
    fi

    printf '%s\n' "python3 or python is required to generate release notes" >&2
    exit 1
}

read_cargo_package_field() {
    local field_name=${1:?missing Cargo.toml package field}

    "${python_bin}" - "${field_name}" <<'PY'
import sys
import tomllib

field_name = sys.argv[1]

with open("Cargo.toml", "rb") as handle:
    cargo = tomllib.load(handle)

print(cargo["package"][field_name])
PY
}

repo_root="$(git rev-parse --show-toplevel)"
cd "${repo_root}"

python_bin="$(resolve_python)"

package_name="$(read_cargo_package_field "name")"
package_version="$(read_cargo_package_field "version")"

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
    printf '[![build](https://github.com/ChenMiaoi/CRIEW/actions/workflows/ci.yml/badge.svg)](https://github.com/ChenMiaoi/CRIEW/actions/workflows/ci.yml)\n'
    printf '[![crates.io](https://img.shields.io/crates/v/criew?label=latest)](https://crates.io/crates/criew)\n'
    printf '[![docs](https://docs.rs/criew/badge.svg)](https://docs.rs/criew/)\n'
    printf '[![codecov](https://codecov.io/github/ChenMiaoi/CRIEW/graph/badge.svg?token=AH99YLKKPD)](https://codecov.io/github/ChenMiaoi/CRIEW)\n\n'
    printf 'Source release for `%s`.\n\n' "$(git rev-list -n 1 "${tag_name}")"
    printf '### Release Assets\n\n'
    assets_found=0
    while IFS= read -r asset_path; do
        [[ -n "${asset_path}" ]] || continue
        assets_found=1
        asset_name="$(basename "${asset_path}")"
        asset_kind="Standalone binary"
        case "${asset_name}" in
            SHA256SUMS)
                asset_kind="Checksum manifest"
                ;;
            *-src.tar.gz | *-src.zip)
                asset_kind="Source archive"
                ;;
            *.tar.gz | *.zip)
                asset_kind="Binary bundle"
                ;;
        esac
        printf -- '- %s: `%s`\n' "${asset_kind}" "${asset_name}"
    done < <(
        find "${asset_dir}" -maxdepth 1 -type f ! -name "$(basename "${output_file}")" | sort
    )

    if [[ "${assets_found}" -eq 0 ]]; then
        printf -- '- No release assets were found in `%s`.\n' "${asset_dir}"
    fi
    printf '\n'

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

#!/usr/bin/env bash

set -euo pipefail

tag_name=${1:?missing tag name}
build_target=${2:?missing build target}
output_dir=${3:?missing output directory}

repo_root="$(git rev-parse --show-toplevel)"
cd "${repo_root}"

install_if_present() {
    local source_path=${1:?missing source path}
    local mode=${2:?missing file mode}
    local target_path=${3:?missing target path}

    if [[ ! -f "${source_path}" ]]; then
        return
    fi

    install -m "${mode}" "${source_path}" "${target_path}"
}

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

binary_path="${repo_root}/target/${build_target}/release/${package_name}"
if [[ ! -x "${binary_path}" ]]; then
    printf '%s\n' "release binary is missing at ${binary_path}" >&2
    exit 1
fi

if [[ "${output_dir}" = /* ]]; then
    release_root="${output_dir%/}"
else
    release_root="${repo_root}/${output_dir%/}"
fi
asset_prefix="${package_name}-${tag_name}-${build_target}"
bundle_root="${release_root}/bundle-root/${asset_prefix}"
tar_asset="${release_root}/${asset_prefix}.tar.gz"
zip_asset="${release_root}/${asset_prefix}.zip"

rm -rf "${release_root}"
mkdir -p "${bundle_root}" "${bundle_root}/LICENSES"

install -m 0755 "${binary_path}" "${bundle_root}/${package_name}"
install -m 0644 "${repo_root}/LICENSE" "${bundle_root}/LICENSE"
install -m 0644 "${repo_root}/README.md" "${bundle_root}/README.md"
install -m 0644 "${repo_root}/README-zh.md" "${bundle_root}/README-zh.md"
install_if_present \
    "${repo_root}/vendor/b4/COPYING" \
    0644 \
    "${bundle_root}/LICENSES/vendor-b4-COPYING"
install_if_present \
    "${repo_root}/vendor/b4/patatt/COPYING" \
    0644 \
    "${bundle_root}/LICENSES/vendor-b4-patatt-COPYING"

tar -C "${release_root}/bundle-root" -czf "${tar_asset}" "${asset_prefix}"
(
    cd "${release_root}/bundle-root"
    zip -qr "${zip_asset}" "${asset_prefix}"
)

rm -rf "${release_root}/bundle-root"

printf '%s\n' "${tar_asset}"
printf '%s\n' "${zip_asset}"

#!/usr/bin/env bash

set -euo pipefail

tag_name=${1:?missing tag name}
output_dir=${2:?missing output directory}

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

if [[ "${output_dir}" = /* ]]; then
    release_root="${output_dir%/}"
else
    release_root="${repo_root}/${output_dir%/}"
fi
asset_prefix="${package_name}-${tag_name}"
source_root="${release_root}/source-root/${asset_prefix}"
tar_asset="${release_root}/${asset_prefix}-src.tar.gz"
zip_asset="${release_root}/${asset_prefix}-src.zip"

rm -rf "${release_root}"
mkdir -p "${source_root}"

while IFS= read -r -d '' tracked_path; do
    target_path="${source_root}/${tracked_path}"
    mkdir -p "$(dirname "${target_path}")"
    cp -a "${tracked_path}" "${target_path}"
done < <(git ls-files --recurse-submodules -z)

tar -C "${release_root}/source-root" -czf "${tar_asset}" "${asset_prefix}"
(
    cd "${release_root}/source-root"
    zip -qr "${zip_asset}" "${asset_prefix}"
)

rm -rf "${release_root}/source-root"

printf '%s\n' "${tar_asset}"
printf '%s\n' "${zip_asset}"

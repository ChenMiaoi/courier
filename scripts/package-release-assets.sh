#!/usr/bin/env bash

set -euo pipefail

tag_name=${1:?missing tag name}
output_dir=${2:?missing output directory}

resolve_python() {
    if command -v python3 >/dev/null 2>&1; then
        printf '%s\n' "python3"
        return
    fi

    if command -v python >/dev/null 2>&1; then
        printf '%s\n' "python"
        return
    fi

    printf '%s\n' "python3 or python is required to package release assets" >&2
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

create_zip_archive() {
    local archive_path=${1:?missing zip archive path}
    local source_root=${2:?missing source root}
    local root_prefix=${3:?missing archive root prefix}

    "${python_bin}" - "${archive_path}" "${source_root}" "${root_prefix}" <<'PY'
from pathlib import Path
import sys
import zipfile

archive_path = Path(sys.argv[1])
source_root = Path(sys.argv[2])
root_prefix = Path(sys.argv[3])

with zipfile.ZipFile(archive_path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
    for source_path in sorted(source_root.rglob("*")):
        if source_path.is_dir():
            continue

        relative_path = source_path.relative_to(source_root)
        archive.write(source_path, arcname=(root_prefix / relative_path).as_posix())
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
create_zip_archive "${zip_asset}" "${release_root}/source-root/${asset_prefix}" "${asset_prefix}"

rm -rf "${release_root}/source-root"

printf '%s\n' "${tar_asset}"
printf '%s\n' "${zip_asset}"

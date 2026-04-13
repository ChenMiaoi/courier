#!/usr/bin/env bash

set -euo pipefail

build_target=${1:?missing build target}
output_dir=${2-}

resolve_python() {
    if command -v python3 >/dev/null 2>&1; then
        printf '%s\n' "python3"
        return
    fi

    if command -v python >/dev/null 2>&1; then
        printf '%s\n' "python"
        return
    fi

    printf '%s\n' "python3 or python is required to smoke-test release packaging" >&2
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

create_temporary_output_dir() {
    local parent_dir=${1:?missing temp dir parent}

    "${python_bin}" - "${parent_dir}" <<'PY'
from pathlib import Path
import sys
import tempfile

parent_dir = Path(sys.argv[1])
parent_dir.mkdir(parents=True, exist_ok=True)

print(Path(tempfile.mkdtemp(prefix="binary-release-packaging-", dir=parent_dir)).as_posix())
PY
}

repo_root="$(git rev-parse --show-toplevel)"
cd "${repo_root}"

python_bin="$(resolve_python)"
source "./scripts/lib/path-utils.sh"
package_name="$(read_cargo_package_field "name")"
package_version="$(read_cargo_package_field "version")"
tag_name="v${package_version}"

if [[ -n "${output_dir}" ]]; then
    smoke_output_dir="$(
        resolve_path_from_repo_root "${python_bin}" "${repo_root}" "${output_dir}"
    )"
    should_cleanup_output_dir=0
else
    smoke_output_dir="$(
        create_temporary_output_dir "${repo_root}/target/release-packaging-smoke"
    )"
    should_cleanup_output_dir=1
fi

cleanup_smoke_output_dir() {
    if [[ "${should_cleanup_output_dir}" -eq 1 ]] && [[ -d "${smoke_output_dir}" ]]; then
        rm -rf "${smoke_output_dir}"
    fi
}

trap cleanup_smoke_output_dir EXIT

mapfile -t generated_assets < <(
    ./scripts/package-binary-release-assets.sh \
        "${tag_name}" \
        "${build_target}" \
        "${smoke_output_dir}"
)

if [[ "${#generated_assets[@]}" -ne 3 ]]; then
    printf '%s\n' \
        "expected 3 generated release assets, got ${#generated_assets[@]}" \
        >&2
    exit 1
fi

for asset_path in "${generated_assets[@]}"; do
    if [[ ! -f "${asset_path}" ]]; then
        printf '%s\n' "missing generated asset: ${asset_path}" >&2
        exit 1
    fi
done

binary_name="${package_name}"
if [[ "${build_target}" == *windows* ]]; then
    binary_name="${binary_name}.exe"
fi

"${python_bin}" \
    - \
    "${build_target}" \
    "${binary_name}" \
    "${generated_assets[0]}" \
    "${generated_assets[1]}" \
    "${generated_assets[2]}" <<'PY'
from pathlib import Path
import sys
import tarfile
import tomllib
import zipfile

build_target = sys.argv[1]
binary_name = sys.argv[2]
standalone_asset = Path(sys.argv[3])
tar_asset = Path(sys.argv[4])
zip_asset = Path(sys.argv[5])

with open("Cargo.toml", "rb") as handle:
    cargo = tomllib.load(handle)

package_name = cargo["package"]["name"]
package_version = cargo["package"]["version"]
asset_prefix = f"{package_name}-v{package_version}-{build_target}"
standalone_asset_name = asset_prefix

if "windows" in build_target:
    standalone_asset_name = f"{standalone_asset_name}.exe"

if standalone_asset.name != standalone_asset_name:
    raise SystemExit(
        f"unexpected standalone asset name: {standalone_asset.name}"
    )

expected_paths = {
    f"{asset_prefix}/{binary_name}",
    f"{asset_prefix}/LICENSE",
    f"{asset_prefix}/README.md",
    f"{asset_prefix}/README-zh.md",
}

vendor_license_paths = [
    (
        Path("vendor/b4/COPYING"),
        f"{asset_prefix}/LICENSES/vendor-b4-COPYING",
    ),
    (
        Path("vendor/b4/patatt/COPYING"),
        f"{asset_prefix}/LICENSES/vendor-b4-patatt-COPYING",
    ),
]

for source_path, archive_path in vendor_license_paths:
    if source_path.is_file():
        expected_paths.add(archive_path)

with tarfile.open(tar_asset, "r:gz") as archive:
    tar_members = set(archive.getnames())

missing_tar_paths = sorted(expected_paths - tar_members)
if missing_tar_paths:
    raise SystemExit(f"missing tar members: {missing_tar_paths}")

with zipfile.ZipFile(zip_asset) as archive:
    zip_members = set(archive.namelist())

missing_zip_paths = sorted(expected_paths - zip_members)
if missing_zip_paths:
    raise SystemExit(f"missing zip members: {missing_zip_paths}")
PY

printf '%s\n' "binary release packaging smoke test passed for ${build_target}"

#!/usr/bin/env bash

set -euo pipefail

output_file=${1:?missing output file}
asset_dir=${2:-$(dirname "${output_file}")}

resolve_python() {
    if command -v python3 >/dev/null 2>&1; then
        printf '%s\n' "python3"
        return
    fi

    if command -v python >/dev/null 2>&1; then
        printf '%s\n' "python"
        return
    fi

    printf '%s\n' "python3 or python is required to generate release checksums" >&2
    exit 1
}

python_bin="$(resolve_python)"

mkdir -p "$(dirname "${output_file}")"

"${python_bin}" - "${output_file}" "${asset_dir}" <<'PY'
from pathlib import Path
import hashlib
import sys

output_file = Path(sys.argv[1])
asset_dir = Path(sys.argv[2])
output_name = output_file.name

asset_paths = sorted(
    path for path in asset_dir.iterdir()
    if path.is_file() and path.name != output_name and path.name != "release-notes.md"
)

lines = []
for asset_path in asset_paths:
    digest = hashlib.sha256(asset_path.read_bytes()).hexdigest()
    lines.append(f"{digest}  {asset_path.name}")

output_file.write_text("\n".join(lines) + ("\n" if lines else ""), encoding="utf-8")
PY

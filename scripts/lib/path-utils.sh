#!/usr/bin/env bash

resolve_path_from_repo_root() {
    local python_bin=${1:?missing python command}
    local repo_root=${2:?missing repo root}
    local requested_path=${3:?missing requested path}

    # Normalize through Python so Windows Git Bash receives a stable
    # forward-slash path for both drive-letter and UNC absolute paths.
    "${python_bin}" - "${repo_root}" "${requested_path}" <<'PY'
from pathlib import Path
import os
import sys

repo_root = Path(sys.argv[1])
requested_path = sys.argv[2]

if os.path.isabs(requested_path):
    resolved_path = Path(requested_path)
else:
    resolved_path = repo_root / requested_path

print(resolved_path.as_posix())
PY
}

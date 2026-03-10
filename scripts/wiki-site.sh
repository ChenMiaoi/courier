#!/usr/bin/env bash

set -euo pipefail

usage() {
    cat <<'EOF'
Usage: ./scripts/wiki-site.sh <prepare|serve|build> [mkdocs args...]

prepare  Stage docs/wiki into target/wiki-docs without installing MkDocs.
serve    Prepare the wiki, install MkDocs into target/wiki-venv, and start a local server.
build    Prepare the wiki, install MkDocs into target/wiki-venv, and build target/wiki-site.
EOF
}

repo_root="$(git rev-parse --show-toplevel)"
requirements_file="${repo_root}/docs/wiki-requirements.txt"
venv_dir="${repo_root}/target/wiki-venv"
python_bin="${venv_dir}/bin/python3"
wiki_dev_addr="${CRIEW_WIKI_DEV_ADDR:-0.0.0.0:8000}"

prepare_wiki() {
    python3 "${repo_root}/scripts/prepare-wiki-site.py"
}

ensure_mkdocs() {
    if [[ ! -x "${python_bin}" ]]; then
        python3 -m venv "${venv_dir}"
        "${python_bin}" -m pip install --upgrade pip
    fi

    "${python_bin}" -m pip install -r "${requirements_file}"
}

command_name="${1:-}"
if [[ -z "${command_name}" ]]; then
    usage >&2
    exit 1
fi

shift || true

case "${command_name}" in
    prepare)
        prepare_wiki
        ;;
    serve)
        prepare_wiki
        ensure_mkdocs
        exec "${python_bin}" -m mkdocs serve \
            -f "${repo_root}/mkdocs.yml" \
            --dev-addr "${wiki_dev_addr}" \
            "$@"
        ;;
    build)
        prepare_wiki
        ensure_mkdocs
        exec "${python_bin}" -m mkdocs build -f "${repo_root}/mkdocs.yml" --clean "$@"
        ;;
    *)
        usage >&2
        exit 1
        ;;
esac

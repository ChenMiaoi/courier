#!/usr/bin/env bash

set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "${repo_root}"

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
    printf '%s\n' \
        "cargo-llvm-cov is required for coverage checks." \
        "Install it with: cargo install cargo-llvm-cov --locked" \
        >&2
    exit 1
fi

coverage_root="${repo_root}/target/coverage"
html_dir="${coverage_root}/html"
lcov_path="${coverage_root}/lcov.info"

mkdir -p "${coverage_root}"

# Clean coverage artifacts first so the report only reflects the current run.
cargo llvm-cov clean --workspace
cargo llvm-cov --workspace --all-features --html --output-dir "${html_dir}"
cargo llvm-cov report --lcov --output-path "${lcov_path}"

printf '%s\n' "coverage html report: ${html_dir}/index.html"
printf '%s\n' "coverage lcov report: ${lcov_path}"

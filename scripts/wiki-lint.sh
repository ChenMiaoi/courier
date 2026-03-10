#!/usr/bin/env bash

set -euo pipefail

autocorrect_repo="huacnlee/autocorrect"

latest_autocorrect_release() {
    curl --silent "https://api.github.com/repos/${autocorrect_repo}/releases/latest" |
        grep '"tag_name":' |
        sed -E 's/.*"([^"]+)".*/\1/'
}

autocorrect_platform() {
    uname | tr '[:upper:]' '[:lower:]'
}

autocorrect_arch() {
    uname -m | sed 's/x86_64/amd64/'
}

autocorrect_libc_suffix() {
    if ldd --version 2>&1 | grep -q 'musl'; then
        printf '%s\n' '-musl'
        return 0
    fi

    printf '%s\n' ''
}

resolve_autocorrect_bin() {
    if command -v autocorrect >/dev/null 2>&1; then
        command -v autocorrect
        return 0
    fi

    local candidate
    for candidate in \
        "${repo_root}/target/wiki-venv/bin/autocorrect" \
        "/opt/homebrew/bin/autocorrect" \
        "/usr/local/bin/autocorrect" \
        "${HOME}/.autocorrect/bin/autocorrect" \
        "${HOME}/.cargo/bin/autocorrect" \
        "${HOME}/bin/autocorrect" \
        "${HOME}/.local/bin/autocorrect"
    do
        if [[ -x "${candidate}" ]]; then
            printf '%s\n' "${candidate}"
            return 0
        fi
    done

    return 1
}

install_autocorrect_into_target() {
    local install_dir="${repo_root}/target/wiki-venv/bin"
    local install_bin="${install_dir}/autocorrect"
    local version platform arch libc_suffix archive_url temp_dir

    mkdir -p "${install_dir}"

    version="$(latest_autocorrect_release)"
    platform="$(autocorrect_platform)"
    arch="$(autocorrect_arch)"
    libc_suffix="$(autocorrect_libc_suffix)"
    archive_url="https://github.com/${autocorrect_repo}/releases/download/${version}/autocorrect-${platform}${libc_suffix}-${arch}.tar.gz"
    temp_dir="$(mktemp -d "${TMPDIR:-/tmp}/autocorrect-install.XXXXXX")"

    printf '%s\n' "autocorrect is not installed; downloading ${archive_url} into ${install_dir}" >&2
    curl -fsSL -o "${temp_dir}/autocorrect.tar.gz" "${archive_url}"
    tar -xzf "${temp_dir}/autocorrect.tar.gz" -C "${temp_dir}"

    if [[ ! -f "${temp_dir}/autocorrect" ]]; then
        printf '%s\n' "autocorrect archive from ${archive_url} did not contain an autocorrect binary" >&2
        rm -rf "${temp_dir}"
        exit 1
    fi

    mv "${temp_dir}/autocorrect" "${install_bin}"
    chmod 0755 "${install_bin}"
    rm -rf "${temp_dir}"

    printf '%s\n' "${install_bin}"
}

ensure_autocorrect() {
    local autocorrect_bin
    if autocorrect_bin="$(resolve_autocorrect_bin)"; then
        printf '%s\n' "${autocorrect_bin}"
        return 0
    fi

    autocorrect_bin="$(install_autocorrect_into_target)"

    if [[ -x "${autocorrect_bin}" ]]; then
        printf '%s\n' "${autocorrect_bin}"
        return 0
    fi

    printf '%s\n' "autocorrect installation completed but no executable was found in ${repo_root}/target/wiki-venv/bin" >&2
    exit 1
}

repo_root="$(git rev-parse --show-toplevel)"
cd "${repo_root}"

autocorrect_bin="$(ensure_autocorrect)"
exec "${autocorrect_bin}" --lint docs/wiki "$@"

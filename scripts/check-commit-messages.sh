#!/usr/bin/env bash

set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
hook_path="${repo_root}/.githooks/commit-msg"

if [[ ! -x "${hook_path}" ]]; then
    printf '%s\n' "missing executable commit-msg hook at ${hook_path}" >&2
    exit 1
fi

declare -A seen_commits=()
declare -a commits=()

append_commit() {
    local commit_sha=$1

    if [[ -n "${seen_commits[${commit_sha}]:-}" ]]; then
        return
    fi

    seen_commits["${commit_sha}"]=1
    commits+=("${commit_sha}")
}

append_commits_from_revspec() {
    local revspec=$1
    local commit_sha

    while IFS= read -r commit_sha; do
        [[ -n "${commit_sha}" ]] || continue
        append_commit "${commit_sha}"
    done < <(git rev-list --reverse "${revspec}")
}

append_push_event_commits() {
    local event_path=$1
    local commit_sha

    while IFS= read -r commit_sha; do
        [[ -n "${commit_sha}" ]] || continue
        append_commit "${commit_sha}"
    done < <(
        python3 - "${event_path}" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    data = json.load(handle)

for commit in data.get("commits", []):
    commit_id = commit.get("id")
    if commit_id:
        print(commit_id)
PY
    )
}

if (( $# > 0 )); then
    for arg in "$@"; do
        if [[ "${arg}" == *".."* ]]; then
            append_commits_from_revspec "${arg}"
            continue
        fi

        if git rev-parse --verify --quiet "${arg}^{commit}" >/dev/null; then
            append_commit "$(git rev-parse "${arg}^{commit}")"
            continue
        fi

        printf '%s\n' "invalid commit or range: ${arg}" >&2
        exit 1
    done
else
    case "${GITHUB_EVENT_NAME:-}" in
        pull_request)
            if [[ -n "${CI_PR_BASE_SHA:-}" && -n "${CI_PR_HEAD_SHA:-}" ]]; then
                append_commits_from_revspec "${CI_PR_BASE_SHA}..${CI_PR_HEAD_SHA}"
            fi
            ;;
        push)
            if [[ -n "${GITHUB_EVENT_PATH:-}" && -f "${GITHUB_EVENT_PATH}" ]]; then
                append_push_event_commits "${GITHUB_EVENT_PATH}"
            elif [[ -n "${CI_COMMIT_BEFORE:-}" && ! "${CI_COMMIT_BEFORE}" =~ ^0+$ ]]; then
                append_commits_from_revspec "${CI_COMMIT_BEFORE}..${GITHUB_SHA:-HEAD}"
            fi
            ;;
    esac
fi

if (( ${#commits[@]} == 0 )); then
    append_commit "$(git rev-parse HEAD^{commit})"
fi

status=0

for commit_sha in "${commits[@]}"; do
    if ! git cat-file -e "${commit_sha}^{commit}" 2>/dev/null; then
        printf '%s\n' "commit not found in local checkout: ${commit_sha}" >&2
        status=1
        continue
    fi

    commit_subject="$(git show -s --format=%s "${commit_sha}")"
    commit_message_file="$(mktemp)"

    git show -s --format=%B "${commit_sha}" > "${commit_message_file}"

    printf '%s\n' "checking commit ${commit_sha}: ${commit_subject}"
    if ! "${hook_path}" "${commit_message_file}"; then
        status=1
    fi

    rm -f "${commit_message_file}"
done

exit "${status}"

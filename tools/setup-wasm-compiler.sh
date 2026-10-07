#!/usr/bin/env bash
# Reuse a working runner compiler before relying on package mirrors.
set -euo pipefail
: "${RUNNER_TEMP:?RUNNER_TEMP must be set}" "${GITHUB_PATH:?GITHUB_PATH must be set}"
probe_dir=$(mktemp -d "$RUNNER_TEMP/openwebide-compiler.XXXXXX")
trap 'rm -rf "$probe_dir"' EXIT

select_compiler() {
    local candidate compiler
    for candidate in clang clang-20 clang-19 clang-18 clang-17; do
        compiler=$(command -v "$candidate") || continue
        if "$compiler" --target=wasm32-unknown-unknown -x c -c /dev/null -o "$probe_dir/probe.o"; then
            local tools_dir="$RUNNER_TEMP/openwebide-ci-compiler"
            mkdir -p "$tools_dir"
            if [[ "$compiler" != "$tools_dir/clang" ]]; then
                ln -sf "$compiler" "$tools_dir/clang"
            fi
            printf '%s\n' "$tools_dir" >> "$GITHUB_PATH"
            "$compiler" --version
            return 0
        fi
    done
    return 1
}

if select_compiler; then
    exit 0
fi
# Hosted runners are disposable; mirror failures must fail the job promptly.
apt_options=(-o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30)
sudo timeout --kill-after=10s 5m apt-get "${apt_options[@]}" update
sudo timeout --kill-after=10s 5m apt-get "${apt_options[@]}" install -y clang
select_compiler

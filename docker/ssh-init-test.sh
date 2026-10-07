#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
test_dir=$(mktemp -d)
trap 'rm -rf "$test_dir"' EXIT
mkdir "$test_dir/source"
printf 'Host git-alias\n  HostName example.com\n' > "$test_dir/source/config"
echo trusted-host > "$test_dir/source/known_hosts"
echo public-key > "$test_dir/source/id_ed25519.pub"
echo private-key > "$test_dir/source/id_ed25519"
ln -s id_ed25519 "$test_dir/source/unexpected.pub"
bash "$root/docker/ssh-init.sh" "$test_dir/source" "$test_dir/target"
cmp "$test_dir/source/config" "$test_dir/target/config"
cmp "$test_dir/source/known_hosts" "$test_dir/target/known_hosts"
cmp "$test_dir/source/id_ed25519.pub" "$test_dir/target/id_ed25519.pub"
[[ ! -e "$test_dir/target/id_ed25519" && ! -e "$test_dir/target/unexpected.pub" ]]
python3 - "$test_dir/target" <<'PY'
import pathlib, stat, sys
root = pathlib.Path(sys.argv[1])
assert stat.S_IMODE(root.stat().st_mode) == 0o700
assert all(stat.S_IMODE(path.stat().st_mode) == 0o600 for path in root.iterdir())
PY
echo updated-host > "$test_dir/source/known_hosts"
bash "$root/docker/ssh-init.sh" "$test_dir/source" "$test_dir/target"
cmp "$test_dir/source/known_hosts" "$test_dir/target/known_hosts"
bash "$root/docker/ssh-init.sh" "$test_dir/missing" "$test_dir/unused"
[[ ! -e "$test_dir/unused" ]]
echo 'Public SSH configuration tests passed'

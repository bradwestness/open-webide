#!/bin/bash
# Import only public SSH setup; private keys stay in the host agent.
set -euo pipefail
source_dir=${1:-/run/openwebide-ssh}
target_dir=${2:-/root/.ssh}
[[ -d $source_dir ]] || exit 0
install -d -m 700 "$target_dir"
for file in "$source_dir/config" "$source_dir/known_hosts" "$source_dir/"*.pub; do
    [[ -f $file && ! -L $file ]] || continue
    install -m 600 "$file" "$target_dir/${file##*/}"
done

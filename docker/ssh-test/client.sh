#!/bin/bash
set -euo pipefail
export GIT_SSH_COMMAND='ssh -o BatchMode=yes -o StrictHostKeyChecking=yes'
export GIT_TERMINAL_PROMPT=0
[[ ! -f /root/.ssh/id_ed25519 ]]
ssh-add -l
for ((i = 0; i < 50; i++)); do
    if git ls-remote git-alias:/repo.git; then break; fi
    sleep 0.1
done
git clone git-alias:/repo.git /workspace/repo
cd /workspace/repo
git checkout -b main
git config user.name Fixture
git config user.email fixture@example.invalid
echo first > file.txt
git add file.txt
git commit -m first
git push origin main
git clone git-alias:/repo.git /workspace/other
echo second > /workspace/other/file.txt
git -C /workspace/other -c user.name=Fixture -c user.email=fixture@example.invalid commit -am second
git -C /workspace/other push origin main
git pull --rebase origin main
[[ $(cat file.txt) == second ]]
if SSH_AUTH_SOCK=/missing git ls-remote origin > /tmp/no-agent.log 2>&1; then exit 1; fi
grep -F 'Permission denied (publickey)' /tmp/no-agent.log >/dev/null
mv /root/.ssh/known_hosts /root/.ssh/trusted
if git ls-remote origin > /tmp/unknown-host.log 2>&1; then exit 1; fi
grep -F 'Host key verification failed' /tmp/unknown-host.log >/dev/null
[[ ! -e /root/.ssh/known_hosts ]]
# A changed host key is rejected as well as a missing one.
ssh-keygen -q -t ed25519 -N '' -f /tmp/wrong-host
{ printf 'git-ssh-fixture '; cat /tmp/wrong-host.pub; } > /root/.ssh/known_hosts
if git ls-remote origin > /tmp/changed-host.log 2>&1; then exit 1; fi
grep -F 'REMOTE HOST IDENTIFICATION HAS CHANGED' /tmp/changed-host.log >/dev/null
mv /root/.ssh/trusted /root/.ssh/known_hosts
git ls-remote origin
echo 'Container SSH Git push/pull and failure contracts passed'

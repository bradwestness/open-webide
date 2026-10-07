#!/bin/bash
set -euo pipefail
# Disposable test keys exist only in this fixture, never in the application container.
mkdir -p /root/.ssh /run/sshd /shared/public
git init --bare --initial-branch=main /repo.git
ssh-keygen -q -t ed25519 -N '' -f /root/test-key
cp /root/test-key.pub /root/.ssh/authorized_keys
chmod 700 /root/.ssh
chmod 600 /root/.ssh/authorized_keys
cp /root/test-key.pub /shared/public/id_ed25519.pub
printf 'git-ssh-fixture ' > /shared/public/known_hosts
cat /etc/ssh/ssh_host_ed25519_key.pub >> /shared/public/known_hosts
cat > /shared/public/config <<'CONFIG'
Host git-alias
    HostName git-ssh-fixture
    User root
    IdentitiesOnly yes
    IdentityFile ~/.ssh/id_ed25519.pub
CONFIG
eval "$(ssh-agent -a /shared/agent.sock)"
ssh-add /root/test-key
touch /shared/ready
exec /usr/sbin/sshd -D -e -o PasswordAuthentication=no -o PermitRootLogin=prohibit-password

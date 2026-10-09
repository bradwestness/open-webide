#!/bin/sh
set -eu
mkdir -p /run/sshd /home/admin/.ssh
cp /fixture.pub /home/admin/.ssh/authorized_keys
chown -R admin:admin /home/admin/.ssh
chmod 700 /home/admin/.ssh
chmod 600 /home/admin/.ssh/authorized_keys
ssh-keygen -A
exec /usr/sbin/sshd -D -e -o PasswordAuthentication=no -o PermitRootLogin=no

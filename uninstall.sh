#!/usr/bin/env bash
# Restore the router to the config captured before cs326 was installed.
set -euo pipefail
HOST="${1:-cs326}"
read -rp "Restore $HOST to its pre-cs326 config? [y/N] " a
[[ "$a" == [yY] ]] || exit 0
ssh "$HOST" 'set -e
  B=/etc/classnet-backup
  [ -d "$B" ] || { echo "no backup at $B"; exit 1; }
  for f in network wireless firewall dhcp; do cp "$B/$f" /etc/config/$f; done
  rm -rf /etc/classnet /usr/sbin/cs326
  /etc/init.d/network reload; /etc/init.d/dnsmasq restart
  /etc/init.d/firewall restart; wifi reload
  echo "restored from $B"'

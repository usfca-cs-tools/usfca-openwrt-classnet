#!/usr/bin/env bash
# Install the CS 326 classroom network onto the router.
#
#   ./install.sh <ssh-host>      e.g. ./install.sh classroom-router
#
# Safe to re-run: it backs up once, then re-pushes and re-applies.
set -euo pipefail

HOST="${1:-router}"
HERE="$(cd "$(dirname "$0")" && pwd)"

say() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }

CONF="$HERE/etc/classnet/classnet.conf"
# A fresh clone has only the example: the real file is gitignored because it
# holds the Wi-Fi passphrases.
if [ ! -f "$CONF" ]; then
  echo "No etc/classnet/classnet.conf yet. Start from the example:" >&2
  echo >&2
  echo "    cp etc/classnet/classnet.conf.example etc/classnet/classnet.conf" >&2
  echo "    \$EDITOR etc/classnet/classnet.conf" >&2
  echo >&2
  echo "Set CLASS, the two SSIDs and their passphrases." >&2
  exit 1
fi
if grep -q CHANGEME "$CONF"; then
  echo "Set KEY and STAFF_KEY in etc/classnet/classnet.conf first." >&2
  exit 1
fi
for req in CLASS SSID KEY STAFF_SSID STAFF_KEY; do
  grep -qE "^$req=" "$CONF" || { echo "etc/classnet/classnet.conf is missing $req" >&2; exit 1; }
done

say "Checking $HOST"
ssh "$HOST" '. /etc/openwrt_release; echo "$DISTRIB_DESCRIPTION on $(cat /tmp/sysinfo/model)"'

say "Backing up current config"
ssh "$HOST" 'set -e
  B=/etc/classnet-backup
  C=$(sed -n "s/^CLASS=\"\(.*\)\".*/\1/p" /etc/classnet/classnet.conf 2>/dev/null)
  # Only a config with none of our own sections in it is worth keeping as the
  # restore point. Snapshotting an already-configured router would leave
  # uninstall.sh "restoring" to a state that is not the one you started from.
  if [ ! -d "$B" ] && [ -n "$C" ] && grep -q "config zone" /etc/config/firewall 2>/dev/null \
     && uci show firewall 2>/dev/null | grep -q "\.${C}_zone="; then
    echo "this router is already configured; not snapshotting it as pristine"
    echo "(if you have an older backup directory, keep it -- that is the real one)"
  elif [ ! -d "$B" ]; then
    mkdir -p "$B"
    for f in network wireless firewall dhcp; do cp /etc/config/$f "$B/$f"; done
    echo "saved pristine config to $B"
  else
    echo "backup already exists at $B (keeping the original pristine copy)"
  fi'

say "Installing dnsmasq-full (stock dnsmasq is built no-nftset)"
ssh "$HOST" 'set -e
  if dnsmasq --version | grep -q "no-nftset"; then
    cp /etc/config/dhcp /tmp/dhcp.preserve
    apk update >/dev/null 2>&1 || true
    apk add dnsmasq-full
    cp /tmp/dhcp.preserve /etc/config/dhcp
    dnsmasq --version | sed -n 2p
  else
    echo "dnsmasq already has nftset support"
  fi'

say "Pushing files"
# classnet.conf is declarative and ALWAYS installed from the repo -- an earlier
# version kept the router's copy, which silently discarded edited passphrases.
# Only staff-macs.list is preserved, because `classnet staff add` is how MACs get
# there in the first place.
if ssh "$HOST" 'test -f /etc/classnet/classnet.conf' 2>/dev/null; then
  ssh "$HOST" 'cat /etc/classnet/classnet.conf' > /tmp/classnet.conf.router 2>/dev/null || true
  if ! cmp -s /tmp/classnet.conf.router "$HERE/etc/classnet/classnet.conf"; then
    echo "  classnet.conf differs from the router's copy; the repo version wins:"
    # `|| true` matters: under `set -o pipefail` a grep that finds nothing makes
    # the whole assignment fail, and `set -e` then exits the installer here --
    # silently, mid-push, having already said it was going to overwrite.
    for k in CLASS SSID KEY STAFF_SSID STAFF_KEY STAFF_HIDDEN NET PROFILES; do
      val() { grep -E "^$2=" "$1" 2>/dev/null | head -1 | sed -e 's/[^=]*=//' -e 's/[[:space:]]*#.*$//' || true; }
      a=$(val /tmp/classnet.conf.router "$k")
      b=$(val "$HERE/etc/classnet/classnet.conf" "$k")
      [ "$a" = "$b" ] || printf '    %-13s router: %-28s repo: %s\n' \
        "$k" "$a" "$b"
    done
  fi
  rm -f /tmp/classnet.conf.router
fi
ssh "$HOST" 'rm -rf /tmp/classnet-stage && mkdir -p /tmp/classnet-stage'
PUSH=(etc/classnet usr/sbin/classnet usr/sbin/classnet-presence
      etc/init.d/classnet-portal tests/policy-simtest.sh tests/portal-simtest.sh)
[ -x "$HERE/usr/sbin/classnet-portal" ] && PUSH+=(usr/sbin/classnet-portal)
COPYFILE_DISABLE=1 tar -C "$HERE" --exclude='.*' -cf - "${PUSH[@]}" \
  | ssh "$HOST" 'tar -C /tmp/classnet-stage -xf -'
ssh "$HOST" 'set -e
  mkdir -p /etc/classnet/allow.d
  S=/tmp/classnet-stage/etc/classnet
  cp "$S"/allow.d/*.list /etc/classnet/allow.d/
  cp "$S"/deny.list "$S"/static4.list "$S"/garden.list /etc/classnet/
  [ -f "$S/groups.txt" ] && [ ! -f /etc/classnet/groups.txt ] && cp "$S/groups.txt" /etc/classnet/
  # schedule.conf holds your class times and is gitignored like classnet.conf.
  # Without one, attendance has no sessions to report against.
  if [ -f "$S/schedule.conf" ]; then
    cp "$S/schedule.conf" /etc/classnet/schedule.conf
  elif [ ! -f /etc/classnet/schedule.conf ]; then
    echo "no schedule.conf -- attendance will report nothing until you add one"
    echo "  (start from etc/classnet/schedule.conf.example)"
  fi
  cp "$S/classnet.conf" /etc/classnet/classnet.conf
  # `|| true`, not `|| echo 0`: grep -c exits 1 on a count of zero, having
  # already printed its own 0, so the fallback made $n two lines and the
  # test below died with "bad number".
  n=$(grep -cE "^[0-9a-fA-F][0-9a-fA-F]:" /etc/classnet/staff-macs.list 2>/dev/null || true)
  if [ "${n:-0}" -gt 0 ]; then
    echo "keeping staff-macs.list ($n MACs)"
  else
    cp "$S/staff-macs.list" /etc/classnet/staff-macs.list
  fi
  cp /tmp/classnet-stage/usr/sbin/classnet /usr/sbin/classnet
  chmod +x /usr/sbin/classnet
  # the portal binary cannot be overwritten while it is running ("Text file
  # busy"), and a silent failure there leaves the old build in place
  [ -f /tmp/classnet-stage/usr/sbin/classnet-portal ] && /etc/init.d/classnet-portal stop 2>/dev/null
  for f in classnet-presence classnet-portal; do
    [ -f "/tmp/classnet-stage/usr/sbin/$f" ] && { cp "/tmp/classnet-stage/usr/sbin/$f" /usr/sbin/; chmod +x "/usr/sbin/$f"; }
  done
  if [ -f /tmp/classnet-stage/etc/init.d/classnet-portal ]; then
    cp /tmp/classnet-stage/etc/init.d/classnet-portal /etc/init.d/
    chmod +x /etc/init.d/classnet-portal
  fi
  # portal.conf holds the OAuth secret and is filled in by hand -- never clobber
  [ -f /etc/classnet/portal.conf ] || cp /etc/classnet/portal.conf.example /etc/classnet/portal.conf 2>/dev/null || true
  for t in policy-simtest:classnet-simtest portal-simtest:classnet-portaltest; do
    src="/tmp/classnet-stage/tests/${t%%:*}.sh"; dst="/usr/sbin/${t##*:}"
    [ -f "$src" ] && { cp "$src" "$dst"; chmod +x "$dst"; }
  done
  rm -rf /tmp/classnet-stage'

say "Timezone"
# The schedule is written in local wall-clock time, so the router has to agree
# with the room about what time it is. A POSIX TZ string carries the DST rules
# without needing the zoneinfo package.
ssh "$HOST" 'TZ_WANT=$(sed -n "s/^TIMEZONE=\"\(.*\)\".*/\1/p" /etc/classnet/classnet.conf)
  ZN_WANT=$(sed -n "s/^ZONENAME=\"\(.*\)\".*/\1/p" /etc/classnet/classnet.conf)
  if [ -n "$TZ_WANT" ]; then
    uci set system.@system[0].timezone="$TZ_WANT"
    [ -n "$ZN_WANT" ] && uci set system.@system[0].zonename="$ZN_WANT"
    uci commit system && /etc/init.d/system reload >/dev/null 2>&1
  fi
  echo "  router clock: $(date)"'

say "Applying"
ssh "$HOST" 'classnet apply'

say "Sign-in portal"
ssh "$HOST" 'if [ -x /usr/sbin/classnet-portal ]; then
    /etc/init.d/classnet-portal enable >/dev/null 2>&1
    /etc/init.d/classnet-portal restart >/dev/null 2>&1
    sleep 1
    if netstat -ltn 2>/dev/null | grep -q ":8080"; then echo "  running on 192.168.63.1:8080"; else echo "  NOT running -- check logread"; fi
    if grep -q "GOOGLE_CLIENT_ID=\"\"" /etc/classnet/portal.conf 2>/dev/null; then
      echo "  no Google client configured yet -- see /etc/classnet/portal.conf.example"
    fi
  else echo "  not installed (run portal/build.sh first)"; fi'

say "Registration gate"
ssh "$HOST" 'classnet gate'

say "Installing the client simulator (optional, for classnet-simtest)"
ssh "$HOST" 'apk add ip-full kmod-veth >/dev/null 2>&1 && echo "ok" || echo "skipped -- classnet-simtest will not run"'

say "Self-test"
ssh "$HOST" 'classnet test' || true

say "End-to-end policy test (simulated student laptop)"
ssh "$HOST" 'classnet-simtest' || true

say "Wireless as configured"
ssh "$HOST" 'C=$(sed -n "s/^CLASS=\"\(.*\)\".*/\1/p" /etc/classnet/classnet.conf)
  for s in ${C}_2g ${C}s_2g; do
    printf "  SSID %-14s key %-16s %s\n" \
      "$(uci get wireless.$s.ssid)" "$(uci get wireless.$s.key)" \
      "$([ "$(uci get wireless.$s.hidden)" = 1 ] && echo "(hidden)" || echo "(broadcast)")"
  done'

say "Status"
ssh "$HOST" 'classnet status'

cat <<'DONE'

Done. Next:
  ssh $HOST 'classnet staff add <instructor-mac>'
  ssh $HOST 'classnet status'

Toggle the student SSID:
  ssh $HOST 'classnet disable'    /    ssh $HOST 'classnet enable'
Suspend the allowlist mid-class without dropping anyone:
  ssh $HOST 'classnet unlock'     /    ssh $HOST 'classnet lock'
DONE

#!/bin/sh
# Run ON THE ROUTER.  Attaches a simulated student laptop to br-$CLASS in a
# network namespace and asserts the policy end to end -- no real laptop needed.
#
#   scp -O tests/router-simtest.sh cs326:/tmp/ && ssh cs326 sh /tmp/router-simtest.sh
#
# Needs: ip-full, kmod-veth   (apk add ip-full kmod-veth)
# Read the class and subnet from the installed config, so these run unchanged
# on any course's router rather than only the one they were written against.
CLASS=$(sed -n 's/^CLASS="\(.*\)".*/\1/p' /etc/classnet/classnet.conf 2>/dev/null); CLASS=${CLASS:-cs326}
NETP=$(sed -n 's/^NET="\(.*\)".*/\1/p' /etc/classnet/classnet.conf 2>/dev/null); NETP=${NETP:-192.168.63}
SNETP=$(sed -n 's/^STAFF_NET="\(.*\)".*/\1/p' /etc/classnet/classnet.conf 2>/dev/null); SNETP=${SNETP:-192.168.64}
NS=stu; VETH=veth-stu; PEER=veth-stu-p; IP=$NETP.50; GW=$NETP.1; STAFFGW=$SNETP.1
SNS=staff; SVETH=veth-staff; SPEER=veth-staff-p; SIP=192.168.64.50; SGW=$STAFFGW
pass=0; fail=0
ok()   { printf '  ok    %s\n' "$1"; pass=$((pass+1)); }
bad()  { printf '  FAIL  %s\n' "$1"; fail=$((fail+1)); }
R()    { ip netns exec $NS "$@"; }

cleanup() {
	# Registered above only when the gate is on; harmless to attempt otherwise.
	classnet unregister 02:c5:32:60:00:01 >/dev/null 2>&1
	for n in $NS $SNS; do
		ip netns del $n 2>/dev/null
		rm -rf /etc/netns/$n
	done
	ip link del $VETH 2>/dev/null
	ip link del $SVETH 2>/dev/null
}
trap cleanup EXIT

command -v ip >/dev/null && ip netns list >/dev/null 2>&1 || {
	echo "needs ip-full: apk add ip-full kmod-veth"; exit 1; }
modprobe veth 2>/dev/null

cleanup
ip netns add $NS
ip link add $VETH type veth peer name $PEER address 02:c5:32:60:00:01 || { echo "veth unavailable: apk add kmod-veth"; exit 1; }
ip link set $VETH master br-$CLASS up
ip link set $PEER netns $NS
ip -n $NS link set lo up
ip -n $NS link set $PEER up
ip -n $NS addr add $IP/24 dev $PEER
ip -n $NS route add default via $GW
mkdir -p /etc/netns/$NS; echo "nameserver $GW" > /etc/netns/$NS/resolv.conf
MAC=$(ip -n $NS link show $PEER | sed -n 's/.*link\/ether \([0-9a-f:]*\).*/\1/p')
echo "simulated student $IP ($MAC) on br-$CLASS"

# This suite tests the ALLOWLIST, which only applies to a registered device.
# With the gate on, an unregistered client fails every "must flow" assertion
# and the suite reports nine failures that are really the gate working. Register
# for the duration so the two concerns stay separately testable -- the gate
# itself is covered by classnet-portaltest.
if [ -f /etc/classnet/state-gate-on ]; then
	echo "registration gate is on -- registering this client for the run"
	classnet register "$MAC" simtest@usfca.edu simtestclient "Sim Test Client" >/dev/null 2>&1
fi

resolves() { R nslookup "$1" $GW 2>/dev/null | sed -n '/^Name:/,$p' | grep -q '^Address'; }
gets()     { R wget -q -T "${2:-10}" -O /dev/null "$1" 2>/dev/null; }

echo "== names that must resolve =="
for d in github.com api.github.com usf-cs326-f26.github.io index.crates.io unpkg.com \
         doc.rust-lang.org rust-exercises.com www.rust-exercises.com \
         rustlings.rust-lang.org play.rust-lang.org google.github.io; do
	resolves "$d" && ok "$d resolves" || bad "$d should resolve"
done

echo "== names that must NOT resolve =="
for d in chatgpt.com api.anthropic.com api.githubcopilot.com \
         copilot-proxy.githubusercontent.com raw.githubusercontent.com \
         docs.rs zulip.com usfca-cs326-f26.zulipchat.com static.zulipchat.com \
         google.com codeload.github.com; do
	resolves "$d" && bad "$d MUST NOT resolve" || ok "$d refused"
done

echo "== traffic that must flow =="
gets https://github.com/USF-CS326-F26                                && ok "github.com over https" || bad "github.com"
# Distinguish "the router blocked it" from "the site itself is down". Without
# this the suite reports a broken allowlist when the truth is a 404 on GitHub's
# side, and the next hour goes into the wrong system.
if gets https://usf-cs326-f26.github.io/; then
	ok "course website"
elif wget -q -T10 -O /dev/null https://usf-cs326-f26.github.io/ 2>/dev/null; then
	bad "course website blocked by the router (reachable from the router itself)"
else
	bad "course website is DOWN at the source -- not a router problem.
        It does not load from the router either, so check the site itself
        before looking at the allowlist."
fi
gets https://index.crates.io/config.json                              && ok "crates.io index"      || bad "crates.io"
gets https://static.rust-lang.org/dist/channel-rust-stable.toml       && ok "rustup channel"       || bad "rustup"
gets https://unpkg.com/reveal.js@5.0.4/package.json                   && ok "reveal.js (slides)"   || bad "unpkg"
gets https://doc.rust-lang.org/std/                              20   && ok "std library docs"     || bad "doc.rust-lang.org"
gets https://rust-exercises.com/                                 20   && ok "rust-exercises.com"   || bad "rust-exercises.com"
gets https://rustlings.rust-lang.org/                            20   && ok "rustlings"           || bad "rustlings"
gets https://play.rust-lang.org/                                 20   && ok "rust playground"     || bad "play.rust-lang.org"
gets https://google.github.io/comprehensive-rust/                20   && ok "comprehensive-rust"  || bad "google.github.io"

echo "== traffic that must be blocked =="
gets https://1.1.1.1/            6 && bad "1.1.1.1 reachable"        || ok "raw IP 1.1.1.1 blocked"
gets https://140.82.112.22/      6 && bad "Copilot IP reachable"     || ok "Copilot IP blocked"
gets https://142.250.191.78/     6 && bad "google IP reachable"      || ok "google IP blocked"

echo "== hardcoded public DNS is redirected back to the filtered resolver =="
R nslookup chatgpt.com 8.8.8.8 2>&1 | grep -q NXDOMAIN && ok "8.8.8.8 hijacked" || bad "DNS redirect not working"

echo "== DHCP hands out the filtered resolver =="
cat > /tmp/.dhcpprobe <<'P'
#!/bin/sh
[ "$1" = bound ] && echo "$dns"
P
chmod +x /tmp/.dhcpprobe
d=$(R udhcpc -i $PEER -n -q -t 3 -T 2 -s /tmp/.dhcpprobe 2>/dev/null | tail -1)
[ "$d" = "$GW" ] && ok "DHCP offers $GW as DNS" || bad "DHCP offered '$d', expected $GW"
rm -f /tmp/.dhcpprobe

echo "== management access =="
# busybox nc here has no -w, so `sleep` holds stdin open long enough for the
# banner to arrive and `head -c` closes the pipe.
ssh_banner() { sleep 2 | ip netns exec "$1" nc "$2" 22 2>/dev/null | head -c 4 | grep -q SSH; }

for a in $GW $SGW 192.168.1.1; do
	ssh_banner $NS $a && bad "router ssh reachable at $a from cs326" \
	                  || ok "router ssh refused at $a"
done

# ...but a staff laptop must get in, because that is how the router is managed.
ip netns add $SNS
ip link add $SVETH type veth peer name $SPEER
ip link set $SVETH master br-${CLASS}s up
ip link set $SPEER netns $SNS
ip -n $SNS link set lo up
ip -n $SNS link set $SPEER up
ip -n $SNS addr add $SIP/24 dev $SPEER
ip -n $SNS route add default via $SGW
for a in $SGW 192.168.1.1; do
	ssh_banner $SNS $a && ok "router ssh from the staff SSID at $a" \
	                   || bad "router ssh from the staff SSID at $a"
done

echo
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]

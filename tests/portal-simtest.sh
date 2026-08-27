#!/bin/sh
# Run ON THE ROUTER. Exercises the registration gate against a simulated
# student in a network namespace -- no second laptop needed.
#
#   ssh cs326 classnet-portaltest
#
# Leaves the gate exactly as it found it.
CLASS=$(sed -n 's/^CLASS="\(.*\)".*/\1/p' /etc/classnet/classnet.conf 2>/dev/null); CLASS=${CLASS:-cs326}
NETP=$(sed -n 's/^NET="\(.*\)".*/\1/p' /etc/classnet/classnet.conf 2>/dev/null); NETP=${NETP:-192.168.63}
NS=stu2; VETH=veth-p; PEER=veth-p-p; GW=$NETP.1
pass=0; fail=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass+1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail+1)); }
R()   { ip netns exec $NS "$@"; }
gets(){ R wget -q -T "${2:-8}" -O - "$1" 2>/dev/null; }
reach(){ R wget -q -T "${2:-8}" -O /dev/null "$1" 2>/dev/null; }

WASON=0; [ -f /etc/classnet/state-gate-on ] && WASON=1
cleanup() {
	ip netns del $NS 2>/dev/null; ip link del $VETH 2>/dev/null; rm -rf /etc/netns/$NS
	[ -n "${MAC:-}" ] && classnet unregister "$MAC" >/dev/null 2>&1
	[ "$WASON" = 1 ] || classnet gate off >/dev/null 2>&1
}
trap cleanup EXIT

ip netns list >/dev/null 2>&1 || { echo "needs ip-full: apk add ip-full kmod-veth"; exit 1; }
modprobe veth 2>/dev/null
ip netns del $NS 2>/dev/null; ip link del $VETH 2>/dev/null
ip netns add $NS
ip link add $VETH type veth peer name $PEER address 02:c5:32:60:00:02 || { echo "needs kmod-veth"; exit 1; }
ip link set $VETH master br-$CLASS up
ip link set $PEER netns $NS
ip -n $NS link set lo up; ip -n $NS link set $PEER up
ip -n $NS route add default via $GW 2>/dev/null
mkdir -p /etc/netns/$NS; echo "nameserver $GW" > /etc/netns/$NS/resolv.conf
R udhcpc -i $PEER -n -q -t 3 -T 2 >/dev/null 2>&1
MAC=$(ip -n $NS link show $PEER | sed -n 's/.*link\/ether \([0-9a-f:]*\).*/\1/p')
[ -n "$(ip -n $NS -4 addr show $PEER | grep inet)" ] || ip -n $NS addr add 192.168.63.50/24 dev $PEER
echo "simulated student $MAC on br-$CLASS"

classnet gate on >/dev/null

echo "== unregistered: locked out of everything but the garden =="
reach https://github.com/USF-CS326-F26   && bad "github reachable"   || ok "github blocked"
reach https://index.crates.io/config.json && bad "crates reachable"  || ok "crates.io blocked"
reach https://accounts.google.com/        && ok "accounts.google.com reachable (sign-in)" || bad "sign-in host blocked"

echo "== the sign-in garden is exactly wide enough, and no wider =="
resolves(){ R nslookup "$1" $GW 2>&1 | grep -q NXDOMAIN && return 1 || return 0; }
for h in www.google.com accounts.google.com; do
	resolves "$h" && ok "$h resolves (needed to sign in)" || bad "$h must resolve"
done
# The apex was tried and dropped: suffix matching admitted mail.google.com and
# chat.google.com to unregistered devices, and Google Chat is banned in session.
for h in google.com mail.google.com chat.google.com drive.google.com; do
	resolves "$h" && bad "$h MUST NOT resolve (garden too wide)" || ok "$h closed"
done

echo "== the OS connectivity probe must NOT be intercepted =="
# Intercepting it makes macOS flag the network captive, and macOS then refuses
# to let the browser reach the sign-in host -- the pop-up appears and sign-in
# cannot be completed from it. Set CAPTIVE_POPUP=1 to trade that back.
body=$(gets http://captive.apple.com/hotspot-detect.html)
case "$body" in
	*"<TITLE>Success</TITLE>"*) ok "probe reaches Apple, so the OS sees a working network" ;;
	"") bad "probe returned nothing" ;;
	*) bad "probe intercepted -- macOS will block the browser from signing in" ;;
esac
gets http://$GW/ | grep -qi "sign-in" \
	&& ok "the portal answers on port 80, so the address needs no port" \
	|| bad "http://$GW/ does not serve the portal"
PH=$(sed -n 's/^PORTAL_HOST="\(.*\)".*/\1/p' /etc/classnet/classnet.conf 2>/dev/null)
if [ -n "$PH" ]; then
	# By name, not just by address: a hosts record answers AAAA with NODATA,
	# where address=/name/ip answers NXDOMAIN and a musl getaddrinfo then
	# discards the good A record and calls the name unresolvable.
	gets "http://$PH/" | grep -qi "sign-in" \
		&& ok "the portal answers to its name ($PH)" \
		|| bad "$PH does not reach the portal"
fi
case "$(gets http://$GW:8080/api/captive)" in
	*'"captive":true'*) ok "RFC 8908 reports captive:true" ;;
	*) bad "captive API wrong for an unregistered device" ;;
esac

echo "== registering unblocks it, live, with no ruleset reload =="
classnet register "$MAC" simtest@usfca.edu simtestuser "Sim Test" >/dev/null 2>&1
nft list set inet fw4 ${CLASS}_reg 2>/dev/null | grep -q "$MAC" && ok "MAC is in ${CLASS}_reg" || bad "MAC missing from ${CLASS}_reg"
reach https://github.com/USF-CS326-F26 && ok "github now reachable" || bad "github still blocked after registering"
reach https://accounts.google.com/     && bad "garden still open to a registered device" || ok "garden closed once registered"
case "$(gets http://captive.apple.com/hotspot-detect.html)" in
	*"<TITLE>Success</TITLE>"*) ok "probe still reaches Apple once registered" ;;
	*) bad "probe intercepted after registering" ;;
esac
case "$(gets http://$GW:8080/api/captive)" in
	*'"captive":false'*) ok "RFC 8908 reports captive:false" ;;
	*) bad "captive API did not flip" ;;
esac

echo "== the who-is-online page =="
OH=$(sed -n 's/^ONLINE_HOST="\(.*\)".*/\1/p' /etc/classnet/classnet.conf 2>/dev/null)
if [ -n "$OH" ]; then
	# By name: the page is reached by Host header, not by path, so an address
	# that answers proves nothing about the name students are actually given.
	body=$(gets "http://$OH/")
	case "$body" in
		*"Online on"*) ok "$OH serves the who-is-online page" ;;
		*"sign-in"*)   bad "$OH falls through to the sign-in page" ;;
		"")            bad "$OH is unreachable" ;;
		*)             bad "$OH served something unexpected" ;;
	esac
	# The page is read by everyone on the network, so it names people and
	# stops there -- addresses stay in `classnet who`.
	case "$body" in
		*"@usfca.edu"*|*"$MAC"*) bad "the page leaks addresses to the whole class" ;;
		*) ok "the page names people, not addresses" ;;
	esac
	# The same name on the same router must NOT be a way past the sign-in page.
	gets "http://$GW/" | grep -qi "sign-in" \
		&& ok "the sign-in page is unaffected by the online page" \
		|| bad "http://$GW/ no longer serves the portal"
	# The staff half cannot be reached from this namespace, so assert the
	# listener exists at all: that is the part no student-side test can see.
	SIP=$(sed -n 's/^STAFF_NET="\(.*\)".*/\1/p' /etc/classnet/classnet.conf 2>/dev/null)
	if [ -n "$SIP" ]; then
		netstat -ltn 2>/dev/null | grep -q "$SIP.1:8080" \
			&& ok "the portal also listens on the staff address ($SIP.1:8080)" \
			|| bad "no staff listener -- $OH will not answer on the staff SSID"
	fi
else
	echo "  (ONLINE_HOST is empty -- page switched off, nothing to test)"
fi

echo "== the roster knows who it is =="
classnet roster --github 2>/dev/null | grep -qx simtestuser && ok "roster lists simtestuser" || bad "roster missing the registration"

echo "== registering and unregistering never touches anyone else's row =="
others=$(classnet roster --github 2>/dev/null | grep -vx simtestuser | wc -l | tr -d " ")
classnet register 02:c5:32:60:00:09 canary@usfca.edu canaryuser "Canary" >/dev/null 2>&1
classnet unregister 02:c5:32:60:00:09 >/dev/null 2>&1
now=$(classnet roster --github 2>/dev/null | grep -vx simtestuser | wc -l | tr -d " ")
[ "$others" = "$now" ] && ok "other registrations survived a register/unregister cycle" \
                       || bad "row count changed: $others -> $now"
classnet unregister 02:c5:32:60:00:09 2>&1 | grep -q "not registered" \
	&& ok "unregistering an unknown MAC is a no-op" || bad "unknown MAC not handled"

echo "== unregistering puts it back behind the portal =="
classnet unregister "$MAC" >/dev/null 2>&1
reach https://github.com/USF-CS326-F26 && bad "still reachable after unregister" || ok "blocked again"

echo
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]

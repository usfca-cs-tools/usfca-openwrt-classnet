# classnet — a restricted classroom network on OpenWrt

Turns an OpenWrt router into a teaching network: a student Wi-Fi that reaches
only the hosts you allow, a staff Wi-Fi that reaches everything, single-sign-on
registration that ties each laptop to a real student, and an attendance log that
needs nothing installed on the laptop.

Built for [CS 326 at USF](https://github.com/USF-CS326-F26), where the exercises
are written in the room and the network is what makes that practical. Nothing in
it is specific to that course except the contents of two text files.

```
                 ┌── SSID "cs245"        → allowlist only, sign-in required
router ──────────┤
                 └── SSID "cs245-staff"  → full Internet, no sign-in
```

## What a student does

**Once, on each laptop they bring.** They join the class Wi-Fi and a sign-in
sheet opens by itself. They sign in with their university Google account,
approving a short code — on the laptop, or on their phone, whichever is easier.
That is the whole thing, and it takes about thirty seconds.

```
 ┌──────────────────────────────┐
 │  CS 245 sign-in              │
 │                              │
 │  1. Go to google.com/device  │
 │  2. Enter this code:         │
 │                              │
 │      ┌────────────────┐      │
 │      │  WTR-QRL-JBQQ  │      │
 │      └────────────────┘      │
 └──────────────────────────────┘
```

**Every session after that: nothing.** They open their laptop, it joins, and it
is recognised. No command to run, no page to visit, no code to type. Their
laptop's address is bound to them and stays bound.

While they are on it, the network reaches what you allowed and nothing else. A
blocked name fails immediately with "server not found" rather than hanging, so
it reads as a closed door rather than a broken connection.

If they are not on your roster, they are told so by name and sent to you —
they are never asked to type their own GitHub username, because a typo there
binds the wrong identity and does not surface until something silently goes to
the wrong place weeks later.

## What the instructor and TAs do

**Before the term.** Drop in a roster mapping people to GitHub usernames (a
Google Forms export works unchanged) and, optionally, a class list (a Canvas
gradebook export works unchanged). Then:

```sh
$ classnet roster check
  ok         jsmith@example.edu               -> jsmith
  MALFORMED  alopez@example.edu               -> "Ana Lopez"  (expected a bare username)
  NOT FOUND  rpatel@example.edu               -> rpatel-typo

Cross-checking 30 enrolled against the roster

  MISSING    kwong                             Wong, Kim

28 of 30 enrolled have a GitHub username on file
```

That is the whole point of the check: you fix those four rows at your desk
instead of discovering them one student at a time at 2:15 on a Thursday.

**Staff never sign in.** You join `cs245-staff`, which is a separate network
with a separate password and unrestricted access. Nothing about the student
network's rules applies to you. (A staff laptop that has to be on the student
SSID can be allowlisted by MAC instead.)

**During a session**, the two things you actually want to know:

```sh
$ classnet who
  9a:71:c4:0e:2d:88  0:42   usf_user=jsmith  email=jsmith@example.edu  name=Jane Smith  github=jsmith
  4e:22:81:aa:19:03  0:11   usf_user=alopez  email=alopez@example.edu  name=Ana Lopez   github=a-lopez
  aa:bb:cc:dd:ee:ff  0:03   (unregistered -- has not signed in)

$ classnet attendance
Thu 2026-09-03  lec-1    08:00-09:50
  GITHUB           FIRST   LAST    TOTAL
  alopez           08:30   09:44   1:14
  jsmith           08:02   09:44   1:42

Thu 2026-09-03  lec-2    14:40-16:30
  GITHUB           FIRST   LAST    TOTAL
  kwong            14:41   16:20   1:39
```

Attendance is derived from the access point's own association table, so it
costs a student nothing and cannot be forgotten. A closed lid or a roam between
radios is stitched back into one visit rather than reported as two.

**Attendance accumulates over the term** — one log per day, sampled every
minute, reported per *session* rather than per calendar day:

```sh
classnet attendance                       # today's sessions
classnet attendance 2026-09-03            # one date
classnet attendance --all                 # every session on record
classnet attendance --since 2026-10-01
classnet attendance --student jsmith      # one person across the term
classnet export --all > attendance.csv    # for the gradebook
```

```sh
$ classnet attendance --student jsmith
jsmith  (Jane Smith <jsmith@example.edu>)
  DATE         SESSION   FIRST   LAST    TOTAL
  2026-09-03   lec-1     08:02   09:44   1:42
  2026-09-04   lab-1     13:05   14:28   1:23

  2 session(s) attended
```

**When something is wrong mid-class**, three escape hatches, in increasing
order of bluntness:

```sh
classnet debug on   # then `classnet log` names the exact host being refused
classnet unlock     # suspend the allowlist without dropping anyone off the network
classnet disable    # take the student SSID down entirely
```

`classnet register <mac> <email> <github>` registers by hand a laptop that
cannot get through the portal. Keep it within reach for the first session.

## Sessions

Attendance is reported against **class periods**, not calendar days. Without
that, a laptop that happens to be near the router at 9am becomes an attendance
record, and two sections meeting on one day merge into a single blob.

Periods live in `schedule.conf`:

```
lec-1|tue,thu|08:00|09:45
lec-2|tue,thu|14:40|16:25
lab-1|fri|13:00|14:30
lab-2|fri|14:40|16:10
```

`SESSION_BUFFER_MIN` (default 5) extends each period at the end, so students who
linger still count. Sections may overlap; a student is reported under every
session whose window they were connected for.

Work outside the timetable — a makeup, office hours, an exam review — gets its
own window on demand:

```sh
classnet session start office-hours
...
classnet session end
classnet session status
```

These times are the router's **local** wall clock, so `install.sh` sets its
timezone from `TIMEZONE` in `classnet.conf` (a POSIX string, so DST is handled
without the zoneinfo package). A router left on UTC will file every session
under the wrong hours.

Logs are kept for `RETAIN_DAYS`, defaulting to 150 — a USF semester from the
first class through finals, plus a month for grade appeals. Expiry is logged to
syslog rather than happening silently. A session costs a few hundred KB, so this
is a policy choice rather than a space one.

## What it actually does

**Default-deny by hostname, not by IP range.** A dedicated `dnsmasq` instance on
the student network resolves only the names you allow and returns NXDOMAIN for
everything else; each answer it does give is fed into an nftables set that the
forward chain matches on. A name that never resolves never gets an address into
the set, so there is nothing to connect to.

Hostnames rather than CIDRs for a concrete reason: on our network,
`api.githubcopilot.com` is a CNAME to `glb-*.github.com` and resolves inside
`140.82.112.0/20` — GitHub's own published range. Allowlisting GitHub by IP
would have silently re-enabled the AI assistant the course policy bans.

**Sign-in that binds a laptop to a person, once.** A student joining for the
first time is intercepted by a captive portal, signs in with their institutional
Google account, and that laptop's MAC is bound to their identity. From then on
they connect and are recognised silently.

That indirection is necessary because randomised MACs are now the norm — 11 of
the first 12 devices we saw used one. But randomised is not *unstable*: Apple
picks a **Fixed** private address per network on WPA2 (Rotating is the default
only on open networks), so the binding holds across the semester.

**Attendance with nothing on the laptop.** The access point already tracks how
long each station has been associated. `classnet who` reads it. No agent, no
browser tab, no ping.

## Requirements

- A router running OpenWrt 24.10 or newer (developed on 25.12, GL.iNet GL-MT6000)
- `dnsmasq-full` — the stock `dnsmasq` is built `no-nftset` and cannot do this
- Rust, on your machine, to cross-build the portal (pure Rust, no C toolchain)
- A Google Cloud project for the sign-in, if you want registration

`install.sh` handles the package swap.

## Install

```sh
git clone https://github.com/usfca-cs-tools/usfca-openwrt-classnet
cd usfca-openwrt-classnet
cp etc/classnet/classnet.conf.example etc/classnet/classnet.conf
$EDITOR etc/classnet/classnet.conf        # CLASS, SSIDs, passphrases
TARGET=aarch64-unknown-linux-musl ./portal/build.sh
./install.sh <router-ssh-host>
```

`CLASS` is the only structural setting. Set `CLASS="cs245"` and every generated
identifier follows: firewall zones `cs245`/`cs245staff`, nftables sets
`cs245_dns4`, bridge `br-cs245`, UCI sections `cs245_*`. Nothing else needs
renaming.

Re-running `install.sh` is safe. It never overwrites `classnet.conf`,
`portal.conf`, the roster or any student data.

## What students may reach

`etc/classnet/allow.d/` — one file per profile, one hostname per line. Turn
profiles on in `PROFILES`. `core`, `github` and `site` are always on.

| profile | contents |
|---|---|
| `github` | git over https/ssh, the web UI, Copilot explicitly denied |
| `site` | **your course website** — edit this one |
| `rust` | crates.io, static.rust-lang.org, rustup |
| `docs` | doc.rust-lang.org, the playground, rustlings |
| `cdn` | unpkg, jsdelivr, cdnjs, Google Fonts |
| `portal` | captive-portal probes, so laptops stop claiming "no Internet" |
| `chat` | *off* — a class chat server |
| `setup` | *off* — Homebrew and apt, for an install day |

Write your own: copy `60-example.list.off` to `60-python.list`, add `python` to
`PROFILES`, run `classnet apply`.

**Two things to know before you write one.** Entries match subdomains, so
`example.com` admits every `*.example.com` — usually more than you meant; name
exact hosts and put exclusions in `deny.list`, which wins. And when a page
half-loads, `classnet debug on` then `classnet log` names the exact host that
was refused, which beats guessing.

## Running a session

```sh
classnet who                  # who is in the room, and for how long
classnet roster               # everyone who has registered a device
classnet attendance           # today's sessions; also --all/--since/--student
classnet export --all         # the whole term as CSV
classnet session start|end    # an attendance window outside the timetable

classnet enable | disable     # student SSID and its rules on/off
classnet lock | unlock        # suspend the allowlist without dropping anyone
classnet gate on | off        # require registration before the allowlist applies
```

`classnet who` prints the whole identity:

```
  9a:71:c4:0e:2d:88  0:42   usf_user=jsmith  email=jsmith@example.edu  name=Jane Smith  github=jsmith
  aa:bb:cc:dd:ee:ff  0:03   (unregistered -- has not signed in)
```

**`gate` ships off.** Turning it on cuts off every already-connected laptop
until it registers, so it has to be a deliberate act, not something an install
does to a live classroom.

## Registration

Set up a Google OAuth client of type **"TVs and Limited Input devices"** at
console.cloud.google.com, and put the id and secret in `portal.conf`.

That type is not arbitrary: it is the only one that issues a device-flow client,
and the device flow is the only Google flow needing no redirect URI. Google
refuses private-IP redirect URIs, so a portal at `192.168.63.1` could never be
registered as one.

Choose **External** audience unless your Cloud project lives in the same
organisation as your students' accounts — Internal restricts sign-in to the
project's own org, which will lock out every student if the project is in a
different one. `openid`, `email` and `profile` are non-sensitive scopes, so
publishing needs no Google verification review.

The portal checks the signed-in domain **server-side** against `ALLOWED_DOMAIN`,
accepting the domain and any subdomain. That check is the only thing between any
Google account and your network, so it is unit-tested.

### The roster

Google supplies a verified email and name; it cannot supply a GitHub username.
Map them in `roster.csv` and registration is a single step. A **Google Forms
export works unchanged**, header row and all.

```sh
cat responses.csv | ssh router 'cat > /etc/classnet/roster.csv'
ssh router 'classnet roster check'
```

`roster check` validates every row against the GitHub API before anyone shows
up, flagging what a free-text form question actually produces: a profile URL
instead of a handle, a display name, a username GitHub does not have, the same
handle claimed twice. Add a class list at `enrolled.csv` (a Canvas gradebook
export works) and it also names the enrolled students who are **missing** — the
ones who would otherwise be turned away with no warning.

Pipe rather than `scp`: OpenWrt has no sftp-server.

## Testing

```sh
ssh router 'classnet test'          # resolver, ruleset, sets
ssh router 'classnet-simtest'       # full policy suite, ~40 assertions
ssh router 'classnet-portaltest'    # registration gate and portal, ~20
bash tests/client-selftest.sh       # from a real laptop on the student SSID
```

The two router suites attach a simulated client to the student bridge in a
network namespace, so the whole policy is testable without a second machine.
They need `ip-full` and `kmod-veth`, which `install.sh` adds.

## What this does not stop

1. **A phone hotspot defeats it completely.** No router configuration detects a
   second interface. Treat the network as a convenience that saves students from
   having to resist a browser tab, not as enforcement.
2. **Allowing a domain allows its subdomains.** `github.com` admits every
   `*.github.com`, so anything unwanted under an allowed parent needs naming in
   `deny.list`.
3. **Shared CDN addresses.** Allowing crates.io also reaches whatever else sits
   on that Fastly edge for a client willing to edit `/etc/hosts`.
4. **A registered MAC is a shared credential.** Sign-in authenticates the
   binding, not each later session, and a laptop on the network is not proof a
   person is in the room. Do not treat the attendance log as if it were.

## Before you turn it on

If you collect attendance, say so first. Students should know what is recorded
— name, institutional email, GitHub username, device address, and connection
times — that it is not scored, and that it is deleted at the end of term. Our
course discloses it in the syllabus alongside everything else the tooling
records, and that ordering matters: disclose, then collect.

## Layout

```
etc/classnet/classnet.conf     CLASS, SSIDs, passphrases, subnets, profiles
etc/classnet/allow.d/          the allowlist, one profile per file
etc/classnet/deny.list         forced NXDOMAIN; beats any broader allow entry
etc/classnet/garden.list       what an unregistered device may reach (sign-in only)
etc/classnet/roster.csv        identity -> GitHub username
etc/classnet/enrolled.csv      the class list, for the missing-students check
etc/classnet/schedule.conf     class periods, for per-session attendance
etc/classnet/attendance/       one append-only log per day
usr/sbin/classnet              the CLI
portal/                        the sign-in portal (Rust, no dependencies)
bin/classnet-release           push a git release into selected students' repos
tests/                         two end-to-end suites
```

`classnet-release` assumes a course where each student has a repository named
`<prefix>-<github-username>` that you collaborate on. If that is not your setup,
ignore it; nothing else depends on it.

## Licence

MIT.

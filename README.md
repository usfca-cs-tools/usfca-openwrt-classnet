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

**Once, on each laptop they bring.** They join the class Wi-Fi and open the
sign-in page in their own browser. One button, their university Google account,
done — about thirty seconds, entirely on the laptop in front of them.

```
Join cs245, then open  http://signin.cs245
```

```
 ┌────────────────────────────────┐
 │  CS 245 sign-in                │
 │                                │
 │      ┌──────────────────┐      │
 │      │  WTR-QRL-JBQQ    │      │
 │      └──────────────────┘      │
 │                                │
 │  ┌──────────────────────────┐  │
 │  │   Sign in with Google    │  │
 │  └──────────────────────────┘  │
 └────────────────────────────────┘
```

**No second device is needed**, and no app. They read the code, open their own
browser, and approve at Google — where they are already signed in, so it is an
account picker rather than a password.

**Why a URL rather than the Wi-Fi pop-up?** Making that pop-up appear means
intercepting the OS connectivity probe, and macOS then flags the network
captive and refuses to let the browser out at all — so the pop-up shows up and
sign-in cannot be completed from it. A URL on a slide costs one line; a sign-in
that cannot be completed costs the session.

`PORTAL_HOST` sets that name; the router answers for it on the class network
only, the bare first label works too, and its IP address always works.

`CAPTIVE_POPUP="1"` trades back, and both were tried on real hardware before
choosing. With the pop-up on, macOS users must complete sign-in inside the
sheet, which has no saved Google session — a full username, password and MFA on
every device — and cannot open a second window, so the sign-in link does
nothing and the code vanishes when followed. Auto-discovery is not worth that.

The exchange with Google runs on the router, not in the page, so a student who
closes the sheet or wanders off mid-sign-in is still let through the moment they
approve. Nothing depends on that window staying open.

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

**Who else is here.** One page the whole room can read:

```
Open  http://online.cs245
```

```
 ┌────────────────────────────────┐
 │  Online on cs245               │
 │                                │
 │  3 people connected to cs245   │
 │  right now.                    │
 │                                │
 │  Ana Lopez            a-lopez  │
 │  Jane Smith            jsmith  │
 │  Kim Wong               kwong  │
 │                                │
 │  1 device here has not         │
 │  signed in.                    │
 └────────────────────────────────┘
```

Names and GitHub usernames, sorted alphabetically, refreshed every thirty
seconds — enough to find the person whose repository you are looking at, or to
see that your project partner is in the room. **People, not devices**: a laptop
and a phone signed in as the same student are one line, and nothing that is not
a name (email, MAC, how long anyone has been sitting there) appears on it. That
detail stays in `classnet who`, which only you can run.

The same page answers on both class networks, on their own side of the router,
so a TA on `cs245-staff` reads it at the same address without joining the
student SSID. `ONLINE_HOST=""` switches the page, its name and its firewall
rules off together.

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

### Closing the network between classes

By default `schedule.conf` only decides how attendance is **reported** — the
student SSID is up until somebody takes it down. `classnet schedule on` makes
the same periods decide whether the network exists at all:

```sh
classnet schedule on      # cs245 is up during class periods, down between them
classnet schedule         # what it would do right now, and what the SSID is
classnet schedule off     # back to the manual switch
```

Armed, a one-minute cron job holds the SSID up inside any period in
`schedule.conf` — plus `SCHEDULE_LEAD_MIN` (default 5) before it starts, so
students who arrive early can connect, and through `SESSION_BUFFER_MIN` after
it ends, so nobody is dropped while they still count as present. Outside those
windows it is down.

**This inverts the one safety property the manual switch has.** `classnet
disable` is an instructor deciding to drop the room; under a schedule the clock
decides, and every laptop still associated at the end of the window goes with
it. That is the point of the feature and also its whole risk, so it ships off,
it announces each transition to syslog with a count of the devices dropped, and
`classnet log` shows those lines next to the wifi events themselves:

```
classnet: schedule: opening cs245 for 'lec-1'
classnet: schedule: closing cs245, no session in progress -- 24 device(s) dropped
```

**Working outside the timetable.** An ad-hoc session is the override, and it
now does what its name always sounded like it did — it opens the network as
well as the attendance window, immediately rather than at the next tick:

```sh
classnet session start office-hours   # SSID up, attendance recording
classnet session end                  # both close together
```

`classnet enable` still works while a schedule is armed, but the cron re-decides
within the minute, so it will be undone; the command says so when it runs. To
hold the network open outside the timetable, start a session.

**If the router reboots mid-class** — a power cut is the case that matters —
it comes back with the SSID in the state it was left in, so a class that was
running is running again as soon as the radios are up. What it will *not* do is
immediately start deciding, and that is deliberate: these boards have no RTC,
and at boot `sysfixtime` restores the clock from the newest mtime under `/etc`,
so the router believes it is whenever it last wrote a file — stale by exactly
the length of the outage. A timetable read off that clock could take the SSID
down in the middle of the period it just came back for.

So the schedule holds until `ntpd` confirms the time, and acts the moment it
does. That is the same gate `dnsmasq` uses before it will trust a DNSSEC
signature (`/etc/hotplug.d/ntp/25-dnsmasqsec`, next to ours). Holding means
leaving the SSID exactly as the reboot left it, which for an outage during
class is the state the room wants anyway. The wait is announced once per boot:

```
classnet: schedule: waiting for ntpd before touching cs245 (no RTC on this board)
classnet: clock confirmed by ntpd -- schedule is live
```

If the outage also took your uplink down, `ntpd` cannot sync and the schedule
stays held until it can — the network keeps whatever state it had rather than
guessing. `classnet schedule` reports the hold, so it is visible rather than
mysterious.

The windows come from `classnet session windows`, which is the same
`sessions_for` the attendance report is built on — one source, so the network
cannot be open at a time that would not be counted, or shut during a time that
would.

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
classnet schedule on | off    # SSID follows schedule.conf instead of staying up
```

`classnet who` prints the whole identity:

```
  9a:71:c4:0e:2d:88  0:42   usf_user=jsmith  email=jsmith@example.edu  name=Jane Smith  github=jsmith
  aa:bb:cc:dd:ee:ff  0:03   (unregistered -- has not signed in)
```

`http://online.cs245` is the same question asked by the room: names only, no
addresses and no durations, readable from either SSID. It is the one page here
that is not about the device asking for it, so it needs no sign-in and works
whether the registration gate is on or off — unregistered devices are counted
at the bottom rather than left out, since a list that silently omits people
reads as a complete one.

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

### Your institution's single sign-on

**Do not skip this.** A university Google account is usually federated: choosing
it at Google redirects to your own identity provider, and if that host is not in
the walled garden the student lands on *"Safari can't find the server"* holding
a screenful of SAML query string. Here the chain runs

```
accounts.google.com  →  idp.usfca.edu (Shibboleth)  →  usfcas.usfca.edu (CAS)  →  Duo
```

so `garden.list` allows the whole institution domain rather than naming each
host — every hop is otherwise a separate outage waiting to happen. Edit that
section for your own institution.

It is easy to miss, because an instructor testing in a browser that already has
a live Google session never takes the SAML path at all. Every student on day one
does. To find yours: `classnet debug on`, attempt a sign-in, then `classnet log`
names every host that was refused.

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
ssh router 'classnet-portaltest'    # registration gate and portal, ~25
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

The who-is-online page is part of that disclosure and not a footnote to it:
anyone who can join the class Wi-Fi can read who else is on it, by name. That
is the point of the page, and it is also the whole of the privacy question, so
decide it deliberately rather than by leaving a default alone. A class where
that is not wanted sets `ONLINE_HOST=""` and the page does not exist.

## Layout

```
etc/classnet/classnet.conf     CLASS, SSIDs, passphrases, subnets, profiles
etc/classnet/allow.d/          the allowlist, one profile per file
etc/classnet/deny.list         forced NXDOMAIN; beats any broader allow entry
etc/classnet/garden.list       what an unregistered device may reach (sign-in only)
etc/classnet/roster.csv        identity -> GitHub username
etc/classnet/enrolled.csv      the class list, for the missing-students check
etc/classnet/schedule.conf     class periods: attendance, and optionally the SSID
etc/classnet/attendance/       one append-only log per day
usr/sbin/classnet              the CLI
usr/sbin/classnet-presence     one attendance sample, per minute from cron
usr/sbin/classnet-schedule     opens and closes the SSID on the timetable, per minute
etc/hotplug.d/ntp/25-classnet  holds the schedule until ntpd confirms the clock
portal/                        the sign-in portal (Rust, no dependencies)
bin/classnet-release           push a git release into selected students' repos
tests/                         two end-to-end suites
```

`classnet-release` assumes a course where each student has a repository named
`<prefix>-<github-username>` that you collaborate on. If that is not your setup,
ignore it; nothing else depends on it.

## Licence

MIT.

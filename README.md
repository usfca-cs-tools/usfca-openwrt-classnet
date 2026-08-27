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
git clone https://github.com/usfca-cs-tools/usfcs-openwrt-manager
cd usfcs-openwrt-manager
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
classnet attendance [date]    # sessions stitched into intervals
classnet export [date]        # the same as CSV

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

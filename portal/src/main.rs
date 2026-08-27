//! cs326-portal -- the CS 326 classroom sign-in portal.
//!
//! A student joining `cs326` for the first time is intercepted here, signs in
//! with their USF Google account, names their GitHub username, and their MAC is
//! bound to that identity. From then on the device is recognised silently and
//! never sees this page again: presence is read from the AP's association
//! table, not from anything the student runs.
//!
//! Two deliberate shapes:
//!
//! * **No dependencies.** TLS is delegated to `uclient-fetch`, which is in
//!   OpenWrt base and already trusts the system CA bundle. That keeps this a
//!   pure-Rust binary that `rust-lld` can link with no C toolchain.
//! * **Google's device flow**, not a redirect. Google refuses private-IP
//!   redirect URIs, so a portal at 192.168.63.1 could never be one. The device
//!   flow needs no redirect URI at all.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const CONF_DIR: &str = "/etc/classnet";
const DEVICE_URL: &str = "https://oauth2.googleapis.com/device/code";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const USERINFO_URL: &str = "https://openidconnect.googleapis.com/v1/userinfo";

// ---------------------------------------------------------------- config ---

struct Config {
    client_id: String,
    client_secret: String,
    domain: String,
    port: u16,
    listen: String,
    /// Mirrors CAPTIVE_POPUP. With the pop-up on, macOS will not let the
    /// student's own browser out, so telling them to use it is bad advice.
    popup: bool,
    /// The course code. Names the nftables set this unblocks, and must match
    /// what the `classnet` CLI generated -- they are two halves of one system.
    class: String,
}

fn load_config() -> Config {
    let mut m = HashMap::new();
    // classnet.conf first for CLASS and the subnet, then portal.conf, which
    // holds the OAuth client and wins on any key it repeats.
    for f in ["classnet.conf", "portal.conf"] {
    if let Ok(txt) = fs::read_to_string(format!("{CONF_DIR}/{f}")) {
        for line in txt.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                m.insert(
                    k.trim().to_string(),
                    v.trim().split('#').next().unwrap_or("").trim().trim_matches('"').to_string(),
                );
            }
        }
    }
    }
    let class = m.remove("CLASS").unwrap_or_else(|| "class".into());
    let popup = m.remove("CAPTIVE_POPUP").as_deref() == Some("1");
    let net = m.remove("NET").unwrap_or_else(|| "192.168.63".into());
    Config {
        client_id: m.remove("GOOGLE_CLIENT_ID").unwrap_or_default(),
        client_secret: m.remove("GOOGLE_CLIENT_SECRET").unwrap_or_default(),
        domain: m.remove("ALLOWED_DOMAIN").unwrap_or_else(|| "usfca.edu".into()),
        port: m.remove("PORTAL_PORT").and_then(|s| s.parse().ok()).unwrap_or(8080),
        listen: m.remove("PORTAL_LISTEN").unwrap_or_else(|| format!("{net}.1")),
        class,
        popup,
    }
}

// ------------------------------------------------------------ tiny helpers --

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Pull a string or number field out of a known-shape JSON response. These are
/// Google and GitHub API replies with flat, predictable bodies -- a real parser
/// would be a dependency for no gain.
fn jget(body: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\"");
    let start = body.find(&pat)? + pat.len();
    let rest = &body[start..];
    let colon = rest.find(':')? + 1;
    let rest = rest[colon..].trim_start();
    if let Some(stripped) = rest.strip_prefix('"') {
        let end = stripped.find('"')?;
        Some(stripped[..end].to_string())
    } else {
        let end = rest.find([',', '}', '\n']).unwrap_or(rest.len());
        let v = rest[..end].trim();
        if v.is_empty() { None } else { Some(v.to_string()) }
    }
}

/// HTTPS via uclient-fetch. Returns the body, or None when the server answered
/// with an HTTP error -- which is exactly what device-flow polling needs, since
/// "not approved yet" arrives as a 428 and success as a 200.
fn https(url: &str, post: Option<&str>) -> Option<String> {
    fetch(url, post).0
}

/// Returns the body on success, and on failure the diagnostic uclient-fetch
/// printed. The second half matters for the roster check: a 403 from GitHub's
/// unauthenticated rate limit is indistinguishable from a 404 if all you have
/// is "it failed", and reporting a rate limit as "no such user" would have the
/// instructor chasing typos that are not there.
fn fetch(url: &str, post: Option<&str>) -> (Option<String>, String) {
    let tmp = format!("/tmp/.cs326-portal-{}-{:?}", std::process::id(),
                      std::thread::current().id());
    let mut cmd = Command::new("uclient-fetch");
    cmd.arg("-T").arg("15").arg("-O").arg(&tmp);
    if let Some(data) = post {
        cmd.arg(format!("--post-data={data}"));
    }
    cmd.arg(url);
    let out = cmd.output();
    let (ok, err) = match &out {
        Ok(o) => (o.status.success(), String::from_utf8_lossy(&o.stderr).to_string()),
        Err(e) => (false, e.to_string()),
    };
    let body = fs::read_to_string(&tmp).ok();
    let _ = fs::remove_file(&tmp);
    if ok { (body, String::new()) } else { (None, err) }
}

enum GhCheck { Exists, Missing, Unknown(String) }

fn github_user(name: &str) -> GhCheck {
    let (body, err) = fetch(&format!("https://api.github.com/users/{}", urlenc(name)), None);
    if body.is_some() {
        return GhCheck::Exists;
    }
    if err.contains("404") {
        GhCheck::Missing
    } else if err.contains("403") || err.contains("429") {
        GhCheck::Unknown("GitHub rate limit -- wait an hour and re-run".into())
    } else {
        GhCheck::Unknown(err.lines().last().unwrap_or("unreachable").trim().to_string())
    }
}

/// Does this address belong to the allowed domain, or a subdomain of it?
///
/// Checked here, server-side, rather than trusting the `hd` request hint --
/// that parameter is a UI suggestion, not a guarantee. It carries real weight:
/// the OAuth client is registered "External" (it has to be, so that student
/// accounts in a different Workspace than the project can sign in at all), so
/// this is the only thing between any Google account on earth and the class
/// network.
fn domain_ok(email: &str, allowed: &str) -> bool {
    let Some(d) = email.rsplit('@').next() else { return false };
    if d.is_empty() || d.len() == email.len() {
        return false; // no '@' at all
    }
    let d = d.to_ascii_lowercase();
    let allowed = allowed.to_ascii_lowercase();
    d == allowed || d.ends_with(&format!(".{allowed}"))
}

fn urlenc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

// ------------------------------------------------------------- mac lookup ---

/// Map the peer IP back to a MAC. The DHCP lease file is authoritative for this
/// subnet; the neighbour table catches a client that set a static address.
fn mac_for_ip(ip: &str) -> Option<String> {
    for path in ["/tmp/dhcp.cs326.leases", "/tmp/dhcp.leases"] {
        if let Ok(txt) = fs::read_to_string(path) {
            for line in txt.lines() {
                let f: Vec<&str> = line.split_whitespace().collect();
                if f.len() >= 3 && f[2] == ip {
                    return Some(f[1].to_lowercase());
                }
            }
        }
    }
    let out = Command::new("ip").args(["neigh", "show", ip]).output().ok()?;
    let txt = String::from_utf8_lossy(&out.stdout);
    let f: Vec<&str> = txt.split_whitespace().collect();
    f.iter().position(|w| *w == "lladdr").and_then(|i| f.get(i + 1)).map(|m| m.to_lowercase())
}

/// Split one CSV line, honouring double quotes -- a Google Forms export quotes
/// any field containing a comma, and full names routinely do.
fn csv_fields(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut cur, mut quoted) = (String::new(), false);
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => { out.push(cur.trim().to_string()); cur.clear(); }
            _ => cur.push(c),
        }
    }
    out.push(cur.trim().to_string());
    out
}

/// Find this person's GitHub username in the roster.
///
/// Takes a hand-written `usf,github` file or a Google Forms export unchanged:
/// if a header row is present the columns are located by name, otherwise it
/// falls back to first-column-is-USF, last-column-is-GitHub. Matches on the
/// full address or on just the local part, so `benson` and `benson@usfca.edu`
/// both find the same row.
fn roster_lookup(email: &str) -> Option<String> {
    roster_find(&fs::read_to_string(format!("{CONF_DIR}/roster.csv")).ok()?, email)
}

fn roster_find(txt: &str, email: &str) -> Option<String> {
    let local = email.split('@').next().unwrap_or(email).to_ascii_lowercase();
    let email = email.to_ascii_lowercase();
    for (who, gh) in roster_rows(txt) {
        let w = who.to_ascii_lowercase();
        if w == email || w == local || w.split('@').next().unwrap_or("") == local {
            return Some(gh);
        }
    }
    None
}

/// Every (usf, github) pair in the file, in order. One parser, shared by the
/// live lookup and `cs326 roster check`, so a roster that checks clean cannot
/// then fail at the door.
fn roster_rows(txt: &str) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let mut usf_col = 0usize;
    let mut gh_col = usize::MAX;
    let mut first = true;

    for line in txt.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f = csv_fields(line);
        if f.len() < 2 {
            continue;
        }

        if first {
            first = false;
            let lower: Vec<String> = f.iter().map(|x| x.to_ascii_lowercase()).collect();
            let is_header = lower.iter().any(|h| h.contains("github"));
            if is_header {
                for (i, h) in lower.iter().enumerate() {
                    if h.contains("github") {
                        gh_col = i;
                    } else if h.contains("email") || h.contains("user") || h.contains("usf") {
                        usf_col = i;
                    }
                }
                continue; // header consumed, not a data row
            }
        }
        let gh_idx = if gh_col == usize::MAX { f.len() - 1 } else { gh_col };
        let (Some(who), Some(gh)) = (f.get(usf_col), f.get(gh_idx)) else { continue };
        if who.is_empty() || gh.is_empty() {
            continue;
        }
        rows.push((who.trim().to_string(), gh.trim().to_string()));
    }
    rows
}

fn is_registered(mac: &str) -> Option<(String, String, String)> {
    let txt = fs::read_to_string(format!("{CONF_DIR}/registrations.tsv")).ok()?;
    for line in txt.lines() {
        if line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() >= 4 && f[0].eq_ignore_ascii_case(mac) {
            return Some((f[1].into(), f[2].into(), f[3].into()));
        }
    }
    None
}

/// Persist the binding and unblock the device immediately. The `nft add
/// element` is what makes this instant: the set is live, so no ruleset reload
/// happens and nobody already connected is disturbed.
fn register(mac: &str, email: &str, name: &str, github: &str, class: &str) -> std::io::Result<()> {
    let path = format!("{CONF_DIR}/registrations.tsv");
    let mut out = String::new();
    if let Ok(txt) = fs::read_to_string(&path) {
        for line in txt.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            if line.starts_with('#') || f.first().map(|m| !m.eq_ignore_ascii_case(mac)).unwrap_or(true) {
                out.push_str(line);
                out.push('\n');
            }
        }
    } else {
        out.push_str("# mac\temail\tname\tgithub\tregistered_at\n");
    }
    out.push_str(&format!("{mac}\t{email}\t{name}\t{github}\t{}\n", now()));
    fs::write(&path, out)?;

    let macs: String = fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| l.split('\t').next())
        .map(|m| format!("{m}\n"))
        .collect();
    fs::write(format!("{CONF_DIR}/registered.macs"), macs)?;

    let _ = Command::new("nft")
        .args(["add", "element", "inet", "fw4", &format!("{class}_reg"), &format!("{{ {mac} }}")])
        .status();
    Ok(())
}

// ------------------------------------------------------------- flow state ---

#[derive(Clone, Default)]
struct Pending {
    device_code: String,
    user_code: String,
    verify_url: String,
    interval: u64,
    expires_at: u64,
    email: String,
    name: String,
    problem: String,
}

type State = Arc<Mutex<HashMap<String, Pending>>>;

// ----------------------------------------------------------------- pages ---

// Every element carries an explicit colour. The macOS captive-portal sheet is a
// stripped-down WebView that does not reliably honour prefers-color-scheme and
// may impose its own body colour -- inheriting from body left the sign-in code
// rendering dark-on-dark, readable only by selecting it with the mouse.
const CSS: &str = "<style>\
:root{--bg:#f4f6f9;--fg:#12151a;--dim:#5a6472;--card:#ffffff;--edge:#dfe3ea;\
--codebg:#12151a;--codefg:#ffffff;--accent:#2563eb;--accentfg:#ffffff}\
@media(prefers-color-scheme:dark){:root{--bg:#0f1115;--fg:#e9ecf1;--dim:#98a1ae;\
--card:#171a21;--edge:#272c36;--codebg:#000000;--codefg:#ffffff}}\
*{box-sizing:border-box}\
body{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;\
font:16px/1.55 -apple-system,BlinkMacSystemFont,'Segoe UI',system-ui,sans-serif;\
background:var(--bg);color:var(--fg);padding:20px}\
.card{width:100%;max-width:27rem;background:var(--card);color:var(--fg);\
border:1px solid var(--edge);border-radius:14px;padding:26px}\
h1{color:var(--fg);font-size:1.2rem;margin:0 0 .5rem}\
p{color:var(--dim);margin:.6rem 0}\
p.lead{color:var(--fg)}\
ol{color:var(--fg);padding-left:1.2rem;margin:.8rem 0}\
li{margin:.45rem 0}\
.code{display:block;background:var(--codebg);color:var(--codefg);\
font:800 2.1rem/1.25 ui-monospace,SFMono-Regular,Menlo,monospace;\
letter-spacing:.16em;text-align:center;border-radius:10px;padding:18px 10px;\
margin:16px 0;user-select:all;-webkit-user-select:all;word-break:break-all}\
a.btn,button{display:block;width:100%;text-align:center;padding:13px;\
border-radius:9px;background:var(--accent);color:var(--accentfg);font-weight:600;\
text-decoration:none;border:0;font-size:1rem;cursor:pointer;margin-top:12px}\
input{width:100%;padding:12px;border-radius:9px;border:1px solid var(--edge);\
background:var(--card);color:var(--fg);font-size:1rem;margin-top:10px}\
.ok{color:#15803d}.err{color:#b91c1c}\
@media(prefers-color-scheme:dark){.ok{color:#4ade80}.err{color:#f87171}}\
.muted{color:var(--dim);font-size:.85rem}\
</style>";

fn page(title: &str, body: &str) -> String {
    format!("<!doctype html><meta charset=utf-8>\
<meta name=viewport content='width=device-width,initial-scale=1'>\
<title>{}</title>{CSS}<div class=card><h1>{}</h1>{}</div>",
        esc(title), esc(title), body)
}

// ---------------------------------------------------------------- routing ---

fn respond(mut s: TcpStream, code: &str, ctype: &str, body: &str) {
    let _ = write!(
        s,
        "HTTP/1.1 {code}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\
Cache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn handle(mut s: TcpStream, cfg: Arc<Config>, state: State) {
    let peer = match s.peer_addr() {
        Ok(a) => a.ip().to_string(),
        Err(_) => return,
    };
    let mut r = BufReader::new(match s.try_clone() {
        Ok(c) => c,
        Err(_) => return,
    });
    let mut line = String::new();
    if r.read_line(&mut line).is_err() {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();

    let mut len = 0usize;
    let mut ua = String::new();
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
            break;
        }
        let lower = h.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = lower.strip_prefix("user-agent:") {
            ua = v.trim().to_string();
        }
    }
    // Drained rather than parsed: no route takes a body since the GitHub
    // question was retired, but the socket should still close cleanly.
    if len > 0 {
        let mut body = vec![0u8; len.min(4096)];
        let _ = r.read_exact(&mut body);
    }

    let path = target.split('?').next().unwrap_or("/").to_string();
    let mac = mac_for_ip(&peer).unwrap_or_default();
    if path == "/" { println!("  ua: {ua}"); }
    println!("{method} {path} from {peer} mac={} {}", 
        if mac.is_empty() { "?" } else { &mac },
        if mac.is_empty() { "" } else if is_registered(&mac).is_some() { "(registered)" } else { "(unregistered)" });

    // RFC 8908: lets iOS/macOS show a proper sign-in sheet and, once we say
    // captive:false, dismiss it by itself instead of leaving a stuck banner.
    if path == "/api/captive" {
        let captive = is_registered(&mac).is_none();
        return respond(s, "200 OK", "application/captive+json",
            &format!("{{\"captive\":{captive},\"user-portal-url\":\"http://{}:{}/\"}}",
                cfg.listen, cfg.port));
    }

    if mac.is_empty() {
        return respond(s, "200 OK", "text/html",
            &page("CS 326", "<p class=err>Could not identify this device.</p>\
<p class=muted>Disconnect from cs326 and rejoin, then reload.</p>"));
    }

    if let Some((email, name, gh)) = is_registered(&mac) {
        if path == "/" {
            return respond(s, "200 OK", "text/html", &page("You are signed in",
                &format!("<p class=ok>Registered as <b>{}</b></p>\
<p>{}<br>{}</p><p class=muted>Device {}. You will not see this page again on \
this laptop. Nothing else to do &mdash; go ahead and work.</p>",
                    esc(&gh), esc(&name), esc(&email), esc(&mac))));
        }
        // Anything else from a registered device: bounce it home.
        return respond(s, "302 Found", "text/html", "");
    }

    match (method.as_str(), path.as_str()) {
        ("GET", "/") => start_flow(s, cfg, state, mac),
        ("GET", "/status") => poll_flow(s, cfg, state, mac),
        // An intercepted request (the OS probe, or any http:// page) lands here.
        _ => {
            let _ = write!(s, "HTTP/1.1 302 Found\r\nLocation: http://{}:{}/\r\n\
Content-Length: 0\r\nConnection: close\r\n\r\n", cfg.listen, cfg.port);
        }
    }
}

fn start_flow(s: TcpStream, cfg: Arc<Config>, state: State, mac: String) {
    // Signed in, but not registrable. Say so and stop -- without this branch
    // the reload after sign-in falls through and issues a fresh device code,
    // looping the student back to the beginning forever.
    if let Some(p) = state.lock().ok().and_then(|m| m.get(&mac).cloned()) {
        if !p.problem.is_empty() {
            return respond(s, "200 OK", "text/html", &page("Almost there", &format!(
                "<p class=ok>Signed in as {}</p><p class=err>{}</p>\
<p class=lead>Ask the instructor or TA to register this laptop &mdash; it takes \
them a moment and you do not need to do anything else.</p>\
<p class=muted>Device {}</p>", esc(&p.email), esc(&p.problem), esc(&mac))));
        }
    }

    if cfg.client_id.is_empty() {
        return respond(s, "200 OK", "text/html", &page("Not configured",
            "<p class=err>The portal has no Google client configured.</p>\
<p class=muted>Set GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET in \
/etc/cs326/portal.conf on the router.</p>"));
    }

    let existing = state.lock().ok().and_then(|m| m.get(&mac).cloned())
        .filter(|p| p.expires_at > now() + 15 && p.email.is_empty());

    let p = match existing {
        Some(p) => p,
        None => {
            let post = format!("client_id={}&scope={}",
                urlenc(&cfg.client_id), urlenc("email profile openid"));
            println!("requesting a device code for mac={mac}");
        let Some(resp) = https(DEVICE_URL, Some(&post)) else {
            println!("device code request FAILED for mac={mac}");
                return respond(s, "200 OK", "text/html", &page("Sign-in unavailable",
                    "<p class=err>Could not reach Google.</p>\
<p class=muted>Tell the instructor; you can be registered by hand.</p>"));
            };
            let p = Pending {
                device_code: jget(&resp, "device_code").unwrap_or_default(),
                user_code: jget(&resp, "user_code").unwrap_or_default(),
                verify_url: jget(&resp, "verification_url")
                    .or_else(|| jget(&resp, "verification_uri"))
                    .unwrap_or_else(|| "https://www.google.com/device".into()),
                interval: jget(&resp, "interval").and_then(|v| v.parse().ok()).unwrap_or(5),
                expires_at: now() + jget(&resp, "expires_in")
                    .and_then(|v| v.parse::<u64>().ok()).unwrap_or(1800),
                ..Default::default()
            };
            if let Ok(mut m) = state.lock() {
                m.insert(mac.clone(), p.clone());
            }
            complete_in_background(cfg.clone(), state.clone(), mac.clone());
            p
        }
    };

    // Lead with "open this page in your own browser", because that is the one
    // place everything lines up: the code, the button, and an existing Google
    // session. The macOS captive sheet has no Google cookies, so it offers no
    // account to pick and demands a full password entry into a WebView -- and
    // it cannot open a new window, so a target=_blank link is inert there.
    // Port 80 is redirected to this portal, so the bare address works.
    let html = format!(
        "<p class=lead>Sign in with your <b>USF</b> account to join the class \
network. Once per laptop, about thirty seconds.</p>\
<span class=code>{}</span>\
<a class=btn href='{}'{}>Sign in with Google &rarr;</a>\
<p class=muted>Google asks for the code <b>first</b>, and only then which \
account to use &mdash; so enter the code above, press <b>Continue</b>, and \
choose your account on the next screen.</p>\
{}\
<script>setInterval(async()=>{{try{{const r=await fetch('/status');\
const j=await r.json();if(j.state==='done'||j.state==='problem')location.reload();}}\
catch(e){{}}}},{}000);</script>",
        esc(&p.user_code), esc(&p.verify_url),
        // The captive sheet cannot open a second window, so target=_blank is
        // inert there and the button simply does nothing. Navigate in place
        // instead; the code is lost from view, which is one more reason the
        // pop-up is not the default.
        if cfg.popup { "" } else { " target=_blank rel=noopener" },
        if cfg.popup {
            // The pop-up is on, so macOS has flagged the network captive and
            // will not let the student's own browser out. Everything has to
            // happen in this window, including the password.
            "<p class=muted>Sign in <b>in this window</b>. It has no saved \
Google session, so you will be asked for your username and password rather \
than shown an account to pick.</p>".to_string()
        } else {
            format!("<p class=muted><b>No account to choose from?</b> You are in \
the Wi-Fi pop-up window, which has no Google session. Open <b>Safari</b> or \
<b>Chrome</b> and go to <b>{}</b> &mdash; this same page &mdash; and press the \
button there.</p>", esc(&cfg.listen))
        },
        p.interval.max(3));
    respond(s, "200 OK", "text/html", &page("CS 326 sign-in", &html));
}

/// Report where this device stands. Cheap and side-effect free: the exchange
/// with Google happens on a background thread, so a student who navigates away
/// from this page still gets registered.
fn poll_flow(s: TcpStream, _cfg: Arc<Config>, state: State, mac: String) {
    if is_registered(&mac).is_some() {
        return respond(s, "200 OK", "application/json", "{\"state\":\"done\"}");
    }
    let st = match state.lock().ok().and_then(|m| m.get(&mac).cloned()) {
        None => "none",
        Some(p) if !p.problem.is_empty() => "problem",
        Some(p) if p.expires_at < now() => "expired",
        Some(_) => "pending",
    };
    respond(s, "200 OK", "application/json", &format!("{{\"state\":\"{st}\"}}"))
}

/// Poll Google until the device code is approved, then finish registration.
///
/// Runs detached from any browser. The page that started the flow may be
/// closed, or navigated away to Google's own sign-in page -- the network still
/// unblocks. Driving this from the page's JavaScript meant a student who
/// followed the sign-in link in the captive sheet was never registered,
/// because nothing was left polling.
fn complete_in_background(cfg: Arc<Config>, state: State, mac: String) {
    std::thread::spawn(move || {
        let Some(p0) = state.lock().ok().and_then(|m| m.get(&mac).cloned()) else { return };
        let post = format!(
            "client_id={}&client_secret={}&device_code={}&grant_type={}",
            urlenc(&cfg.client_id), urlenc(&cfg.client_secret), urlenc(&p0.device_code),
            urlenc("urn:ietf:params:oauth:grant-type:device_code"));

        while now() < p0.expires_at {
            std::thread::sleep(std::time::Duration::from_secs(p0.interval.max(3)));
            if state.lock().ok().and_then(|m| m.get(&mac).cloned()).is_none() {
                return; // finished or abandoned
            }
            // A miss is "not approved yet": uclient-fetch reports an HTTP error
            // as a failure with no body, which is what Google's 428 looks like.
            let Some(tok) = https(TOKEN_URL, Some(&post)) else { continue };
            let Some(access) = jget(&tok, "access_token") else { continue };
            println!("google approved mac={mac}");

            let Some(info) = https(&format!("{USERINFO_URL}?access_token={}", urlenc(&access)), None)
                else { continue };
            let email = jget(&info, "email").unwrap_or_default();
            let name = jget(&info, "name").unwrap_or_else(|| email.clone());
            let verified = jget(&info, "email_verified").map(|v| v == "true").unwrap_or(false);
            println!("google identity for mac={mac}: {email} verified={verified}");

            let problem = if !verified || !domain_ok(&email, &cfg.domain) {
                format!("{email} is not a {} account.", cfg.domain)
            } else {
                match roster_lookup(&email) {
                    None => {
                        println!("no roster entry for {email} -- cannot register {mac}");
                        format!("We do not have a GitHub username on file for {email}.")
                    }
                    Some(gh) => {
                        if matches!(github_user(&gh), GhCheck::Missing) {
                            format!("The roster lists your GitHub username as \"{gh}\", but GitHub has no such user.")
                        } else {
                            match register(&mac, &email, &name, &gh, &cfg.class) {
                                Ok(()) => {
                                    println!("registered {mac} as {gh} <{email}> from the roster");
                                    if let Ok(mut m) = state.lock() { m.remove(&mac); }
                                    return;
                                }
                                Err(e) => format!("Could not save the registration: {e}"),
                            }
                        }
                    }
                }
            };
            if let Ok(mut m) = state.lock() {
                if let Some(e) = m.get_mut(&mac) { e.email = email; e.name = name; e.problem = problem; }
            }
            return;
        }
        println!("device code expired for mac={mac}");
    });
}
fn enrolled_rows(txt: &str) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let mut id_col = 0usize;
    let mut name_col = usize::MAX;
    let mut first = true;

    for line in txt.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f = csv_fields(line);
        if first {
            first = false;
            let lower: Vec<String> = f.iter().map(|x| x.to_ascii_lowercase()).collect();
            if lower.iter().any(|h| h.contains("login") || h.contains("email")
                                 || h.contains("student") || h.contains("user")) {
                for (i, h) in lower.iter().enumerate() {
                    // "SIS Login ID" is the useful one in a Canvas export;
                    // prefer it over the display name in "Student".
                    if h.contains("login") || h.contains("email") || h.contains("sis user") {
                        id_col = i;
                    } else if h.contains("student") || h.contains("name") {
                        name_col = i;
                    }
                }
                continue;
            }
        }
        let Some(id) = f.get(id_col) else { continue };
        let id = id.trim();
        if id.is_empty() || id.eq_ignore_ascii_case("Points Possible") {
            continue;
        }
        let name = f.get(name_col).map(|n| n.trim().to_string()).unwrap_or_default();
        rows.push((id.to_string(), name));
    }
    rows
}

/// Two identifiers naming the same person. Compares local parts, so a class
/// list holding `benson` or `benson@usfca.edu` matches a roster holding either.
fn same_person(a: &str, b: &str) -> bool {
    let norm = |x: &str| x.split('@').next().unwrap_or(x).trim().to_ascii_lowercase();
    norm(a) == norm(b)
}

/// `cs326 roster check` -- validate the whole file before anyone shows up,
/// rather than discovering bad rows one student at a time at the door.
fn check_roster() -> i32 {
    let path = format!("{CONF_DIR}/roster.csv");
    let Ok(txt) = fs::read_to_string(&path) else {
        println!("no roster at {path}");
        println!("Without one, every student is turned away at the portal and must be");
        println!("registered by hand. See roster.csv.example.");
        return 1;
    };
    let rows = roster_rows(&txt);
    if rows.is_empty() {
        println!("{path} parsed to zero usable rows.");
        println!("Expected two columns: a USF address or username, and a GitHub username.");
        return 1;
    }

    println!("Checking {} row(s) in {path} against GitHub\n", rows.len());
    let (mut ok, mut bad, mut unknown) = (0, 0, 0);
    let mut seen_gh: HashMap<String, String> = HashMap::new();
    let mut seen_usf: HashMap<String, String> = HashMap::new();
    let mut notes: Vec<String> = Vec::new();

    for (who, gh) in &rows {
        let gh_key = gh.to_ascii_lowercase();
        let usf_key = who.to_ascii_lowercase();

        if let Some(prev) = seen_gh.get(&gh_key) {
            notes.push(format!("  DUPLICATE  {gh} is claimed by both {prev} and {who}"));
        }
        if let Some(prev) = seen_usf.get(&usf_key) {
            notes.push(format!("  DUPLICATE  {who} appears twice ({prev} and {gh})"));
        }
        seen_gh.insert(gh_key, who.clone());
        seen_usf.insert(usf_key, gh.clone());

        // A handle, not a URL or a display name -- the usual Google Forms answers.
        if gh.contains('/') || gh.contains(' ') || gh.contains('@') {
            println!("  MALFORMED  {who:<32} -> \"{gh}\"  (expected a bare username)");
            bad += 1;
            continue;
        }

        match github_user(gh) {
            GhCheck::Exists => { println!("  ok         {who:<32} -> {gh}"); ok += 1; }
            GhCheck::Missing => {
                println!("  NOT FOUND  {who:<32} -> {gh}");
                bad += 1;
            }
            GhCheck::Unknown(why) => {
                println!("  ?          {who:<32} -> {gh}  ({why})");
                unknown += 1;
            }
        }
    }
    for n in &notes { println!("{n}"); }

    // Counted separately: a row can be perfectly valid and still be duplicated,
    // so folding the two together produced totals that did not add up.
    println!("\n{} row(s): {ok} verified, {bad} bad{}{}",
        rows.len(),
        if unknown > 0 { format!(", {unknown} unchecked") } else { String::new() },
        if notes.is_empty() { String::new() } else { format!(", {} duplicate warning(s)", notes.len()) });
    if unknown > 0 {
        println!("Unchecked rows were not proven wrong -- re-run once the rate limit clears.");
    }
    let mut missing = 0;
    // Who is enrolled but has no GitHub username on file. These are the students
    // who get turned away at the portal, and the only way to know before a
    // session is to compare against the class list.
    let epath = format!("{CONF_DIR}/enrolled.csv");
    if let Ok(etxt) = fs::read_to_string(&epath) {
        let enrolled = enrolled_rows(&etxt);
        println!("\nCross-checking {} enrolled against the roster ({epath})\n", enrolled.len());
        for (id, name) in &enrolled {
            if !rows.iter().any(|(who, _)| same_person(who, id)) {
                let label = if name.is_empty() { String::new() } else { format!("  {name}") };
                println!("  MISSING    {id:<32}{label}");
                missing += 1;
            }
        }
        for (who, gh) in &rows {
            if !enrolled.iter().any(|(id, _)| same_person(id, who)) {
                println!("  not enrolled  {who:<29} -> {gh}  (staff, or dropped)");
            }
        }
        println!("\n{} of {} enrolled have a GitHub username on file",
            enrolled.len() - missing, enrolled.len());
        if missing > 0 {
            println!("Those students will be turned away at the portal. Chase the form, or");
            println!("register them by hand with: cs326 register <mac> <email> <github>");
        }
    } else {
        println!("\nNo class list at {epath} -- cannot say who is missing.");
        println!("Drop one in (a Canvas export works) to catch students who never");
        println!("filled the form, before they are standing in front of you.");
    }

    if bad > 0 || !notes.is_empty() || missing > 0 { 1 } else { 0 }
}

fn main() {
    if std::env::args().any(|a| a == "--check-roster") {
        std::process::exit(check_roster());
    }
    let cfg = Arc::new(load_config());
    let state: State = Arc::new(Mutex::new(HashMap::new()));
    let bind = format!("{}:{}", cfg.listen, cfg.port);
    let l = match TcpListener::bind(&bind) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cs326-portal: cannot bind {bind}: {e}");
            std::process::exit(1);
        }
    };
    println!("cs326-portal listening on {bind}");
    for stream in l.incoming().flatten() {
        let (cfg, state) = (cfg.clone(), state.clone());
        std::thread::spawn(move || handle(stream, cfg, state));
    }
}


#[cfg(test)]
mod tests {
    use super::{domain_ok, roster_find};

    const FORMS: &str = "\
Timestamp,Email Address,What is your GitHub username?\n\
2026/08/20 9:14:02 AM,jsmith@dons.usfca.edu,jsmith\n\
2026/08/20 9:15:41 AM,\"alopez@dons.usfca.edu\",a-lopez\n";

    const PLAIN: &str = "\
# hand written\n\
benson,gdbenson\n\
rpatel@dons.usfca.edu,rpatel-dev\n";

    #[test]
    fn reads_a_google_forms_export_unchanged() {
        assert_eq!(roster_find(FORMS, "jsmith@dons.usfca.edu").as_deref(), Some("jsmith"));
        // quoted field, and a username with a hyphen
        assert_eq!(roster_find(FORMS, "alopez@dons.usfca.edu").as_deref(), Some("a-lopez"));
        // the header must not be treated as data
        assert_eq!(roster_find(FORMS, "Email Address"), None);
    }

    #[test]
    fn reads_a_plain_two_column_file() {
        // local part in the file, full address signing in
        assert_eq!(roster_find(PLAIN, "benson@usfca.edu").as_deref(), Some("gdbenson"));
        // full address in the file, and case should not matter
        assert_eq!(roster_find(PLAIN, "RPatel@Dons.USFCA.edu").as_deref(), Some("rpatel-dev"));
        assert_eq!(roster_find(PLAIN, "nobody@usfca.edu"), None);
    }

    #[test]
    fn matches_on_the_local_part_across_usf_subdomains() {
        // a student listed under one address signing in under another
        let csv = "usf email,github\njsmith@usfca.edu,jsmith\n";
        assert_eq!(roster_find(csv, "jsmith@dons.usfca.edu").as_deref(), Some("jsmith"));
    }

    #[test]
    fn reads_a_canvas_gradebook_export() {
        use super::enrolled_rows;
        // Canvas puts a "Points Possible" pseudo-row second; it is not a student
        // and reporting it as missing every run would train you to ignore the output.
        let canvas = "\
Student,ID,SIS User ID,SIS Login ID,Section\n\
\"    Points Possible\",,,,\n\
\"Smith, Jane\",12345,20123456,jsmith,CS 326-01\n\
\"Lopez, Ana\",12346,20123457,alopez,CS 326-01\n";
        let rows = enrolled_rows(canvas);
        assert_eq!(rows.len(), 2, "Points Possible must not count as a student");
        assert_eq!(rows[0].0, "jsmith");
        assert_eq!(rows[0].1, "Smith, Jane");
    }

    #[test]
    fn a_plain_list_of_addresses_is_a_class_list_too() {
        use super::enrolled_rows;
        let rows = enrolled_rows("jsmith@dons.usfca.edu\nalopez@dons.usfca.edu\n");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].0, "alopez@dons.usfca.edu");
    }

    #[test]
    fn matches_a_class_list_username_to_a_roster_address() {
        use super::same_person;
        assert!(same_person("jsmith", "jsmith@dons.usfca.edu"));
        assert!(same_person("JSmith@usfca.edu", "jsmith@dons.usfca.edu"));
        assert!(!same_person("jsmith", "jsmithe"));
    }

    #[test]
    fn ignores_rows_with_no_github_username() {
        let csv = "email,github\njsmith@dons.usfca.edu,\n";
        assert_eq!(roster_find(csv, "jsmith@dons.usfca.edu"), None);
    }

    #[test]
    fn accepts_the_domain_and_its_subdomains() {
        // Students, staff and the CS Workspace all live under usfca.edu.
        for e in [
            "jsmith@usfca.edu",
            "jsmith@dons.usfca.edu",
            "benson@cs.usfca.edu",
            "JSmith@Dons.USFCA.edu",
        ] {
            assert!(domain_ok(e, "usfca.edu"), "should accept {e}");
        }
    }

    #[test]
    fn rejects_lookalikes_and_outsiders() {
        for e in [
            "someone@gmail.com",
            "someone@usfca.edu.attacker.com",
            "someone@evilusfca.edu",   // ends with the domain but not ".domain"
            "someone@notusfca.edu",
            "usfca.edu",               // no @ at all
            "",
        ] {
            assert!(!domain_ok(e, "usfca.edu"), "should reject {e}");
        }
    }
}

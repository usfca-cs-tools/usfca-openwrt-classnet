#!/usr/bin/env bash
# Run this ON A LAPTOP joined to the `cs326` SSID.
#   bash client-selftest.sh
# Every line should read ok.
pass=0; fail=0
ck() { # ck <label> <expect:yes|no> <command...>
  local label="$1" expect="$2"; shift 2
  if "$@" >/dev/null 2>&1; then got=yes; else got=no; fi
  if [ "$got" = "$expect" ]; then printf '  ok   %s\n' "$label"; pass=$((pass+1))
  else printf '  FAIL %s (expected %s, got %s)\n' "$label" "$expect" "$got"; fail=$((fail+1)); fi
}
resolves() { [ -n "$(dig +short +time=3 +tries=1 "$1" 2>/dev/null)" ]; }
reaches()  { curl -sS -m 8 -o /dev/null "$1"; }

echo "== these must work =="
ck "github.com resolves"                  yes resolves github.com
ck "course site resolves"                 yes resolves usf-cs326-f26.github.io
ck "crates.io index resolves"             yes resolves index.crates.io
ck "git ls-remote course repo"            yes git ls-remote https://github.com/USF-CS326-F26/oslings-course.git
ck "course website loads"                 yes reaches https://usf-cs326-f26.github.io/
ck "reveal.js (slides render)"            yes reaches https://unpkg.com/reveal.js@5.0.4/dist/reveal.js
ck "MathJax"                              yes reaches https://cdn.jsdelivr.net/npm/mathjax@3/es5/tex-mml-chtml.js
ck "crates.io index reachable"            yes reaches https://index.crates.io/config.json
ck "std library docs"                     yes reaches https://doc.rust-lang.org/std/
ck "rust-exercises.com"                   yes reaches https://rust-exercises.com/
ck "rustlings.rust-lang.org"              yes reaches https://rustlings.rust-lang.org/
ck "rust playground"                      yes reaches https://play.rust-lang.org/
ck "Comprehensive Rust"                   yes reaches https://google.github.io/comprehensive-rust/
ck "docs.rs (crate API docs)"             yes reaches https://docs.rs/serde/latest/serde/
ck "rust-lang.org (via www)"              yes reaches https://www.rust-lang.org/
ck "asciinema.org"                        yes reaches https://asciinema.org/

echo "== these must be blocked =="
ck "chatgpt.com"                          no  resolves chatgpt.com
ck "api.anthropic.com"                    no  resolves api.anthropic.com
ck "api.githubcopilot.com (Copilot)"      no  resolves api.githubcopilot.com
ck "copilot-proxy.githubusercontent.com"  no  resolves copilot-proxy.githubusercontent.com
ck "class Zulip"                          no  resolves usfca-cs326-f26.zulipchat.com
ck "raw.githubusercontent.com"            no  resolves raw.githubusercontent.com
ck "google.com"                           no  resolves google.com
ck "direct IP to 1.1.1.1"                 no  reaches https://1.1.1.1/

echo
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]

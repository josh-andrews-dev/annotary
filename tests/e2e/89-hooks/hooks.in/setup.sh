#!/bin/sh
# Member @hookie with profile "hookie" on the harness server (the golden cannot spell its URL)
# and a Document `hk-plan` of its own.
set -e
url=$(sed -n 's/^server = "\(http:\/\/127\.0\.0\.1:[0-9]*\)"$/\1/p' "$HOME/.annotary/config.toml" | grep -v ':1$' | head -n 1)
tok=$(annotary -p rowan member add hookie --name "Hookie" 2>&1 | sed -n 's/^.*ANNOTARY_TOKEN=\(ant_[A-Za-z0-9_-]*\).*$/\1/p' | head -n 1)
annotary -p rowan profile add hookie --server "$url" --token "$tok" >/dev/null 2>&1
annotary -p hookie -q push hk-plan.md --vault notes -m "first" >/dev/null
echo "Hookie has hk-plan"

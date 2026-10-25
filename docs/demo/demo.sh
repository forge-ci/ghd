#!/usr/bin/env bash
# ghd demo: real commands against real GitHub (talos-hq/clud).
export PATH="$HOME/dev/forge-ci/.venv/bin:$PATH"
export GHD_FRESH_MS=400
R=talos-hq/clud
PRS="2135 2133 2132 2131 2128 2135 2133 2132 2131 2128"
say() { printf '\n\033[1;36m# %s\033[0m\n' "$1"; }
cmd() { printf '\033[1;32m$\033[0m \001%s\002\n' "$1"; bash -c "$1"; }
say "ghd: a cached, batched gateway for gh reads"
cmd "ghd --version"
say "cold: the first read starts the daemon and fetches over GraphQL"
cmd "time ghd pr view 2138 -R $R --json number,state,title,mergedAt"
say "10 agents at once: 5 PRs, each asked twice"
cmd "printf '%s\n' $PRS | GHD_DEBUG=1 xargs -P10 -I{} ghd pr view {} -R $R --json state 2>&1 >/dev/null | sort | uniq -c"
cmd "ghd stats"
say "meanwhile, the same read through plain gh"
cmd "time gh pr view 2138 -R $R --json state >/dev/null"
say "the 10 agents ask again: freshness proven by free ETag 304s"
cmd "printf '%s\n' $PRS | GHD_DEBUG=1 xargs -P10 -I{} ghd pr view {} -R $R --json state 2>&1 >/dev/null | sort | uniq -c"
cmd "ghd stats"
say "warm reads: the first revalidates (one REST round trip), the next is served fresh"
cmd "time GHD_DEBUG=1 ghd pr view 2138 -R $R --json state"
cmd "time GHD_DEBUG=1 ghd pr view 2138 -R $R --json state"
say "Python callers share the same cache"
cmd "python -c \"from ghd.bindings import pr_view; print(pr_view('$R', 2138, ['state', 'mergeable']))\""

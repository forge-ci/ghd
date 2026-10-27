---
name: ghd
description: Use an installed ghd agent plugin for repeated GitHub CLI reads and explicit fresh repository snapshots when the user selected ghd for the task.
---

Use the generated `gh` or `gh.cmd` launcher for ordinary GitHub CLI
arguments. Installation's `--launchers DIR` places them in a separate trusted
directory outside the managed root. `GHD_PLUGIN_HOME` must name the verified installation root.
These wrappers preserve streams/status and route through `ghd gh`. A separately
trusted publisher outside that root must be available through `GHD_PUBLISHER`
or trusted system PATH; never use the managed image as its own validator.
Installation is explicit: use the pinned artifact/checksum instructions in
[the plugin README](../../README.md); do not resolve a latest release or
silently download a missing binary during an ordinary call.

Use `ghd snapshot -R owner/repo --fresh` for authority decisions. Ordinary
cached reads are planning observations. Accept stale snapshot output only
when the user explicitly allowed stale planning and retain its age/source.
Check truncation and pageInfo before claiming complete repository state.

Writes still require the user's existing task authorization. Do not replay a
write after a lost daemon reply, bypass an uncertain-write fence, reset a
cooldown, or change authentication to make a read succeed. Preserve explicit
host/repository bindings and native error/status behavior. Explain a freshness
refusal instead of treating an older body as authoritative.

The plugin preference applies only in its selected context. Respect the user's
choice of system gh or connected tools, and use connected tools for capabilities
that ghd/gh does not provide. Native passthrough and external gh calls are
outside the daemon's dispatch governor; do not claim universal quota control.

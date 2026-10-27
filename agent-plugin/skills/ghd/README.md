# Scoped ghd preference skill

The repository requires every source directory README to contain at least fifty lines.
This directory is part of the ghd agent plugin.
See the root plugin README for Python installation and optional launcher generation.
Installation requires an explicit version.
Installation requires a trusted SHA256 pin.
A checksum from an untrusted download location is not independent provenance.
The bootstrap verifies the complete archive.
The publisher verifies the selected binary.
Only a selected regular archive member is admitted.
No archive is extracted wholesale.
No latest-release lookup occurs.
Normal wrapper calls never download tools.
Use an absolute current-user installation root.
Private native permissions protect manifests and binary objects.
The active record switches atomically after validation.
Old records remain usable if validation fails.
A custody lock serializes publication.
Contention is an explicit failure.
Do not delete a live custody lock to bypass contention.
Status checks record and binary integrity.
Upgrade repeats the pinned install flow.
Stop consumers before removing running Windows binaries.
Uninstall removes only the selected tool kind.
Other user files outside owned objects remain untouched.
Empty coordination directories and locks are retained intentionally.
Set GHD_PLUGIN_HOME for wrappers.
Select the generated gh launcher on Unix.
Select the generated gh.cmd launcher on Windows.
Both wrappers preserve original arguments.
Both wrappers preserve standard streams.
Both wrappers preserve the completed child status.
The wrapper delegates to the native ghd gh route.
Explicit GHD_GH_PATH selects system gh.
Opt-in GHD_MANAGED_GH_DIR selects managed gh.
Shim markers prevent recursive discovery.
Authentication is resolved by the existing identity layer.
Credentials do not belong in plugin files.
Cached reads are planning observations.
Use fresh snapshots for authority decisions.
Inspect pageInfo and truncation.
Stale planning needs explicit consent.
Writes still need task authorization.
A lost write reply is never replayed.
Uncertain write fences must be preserved.
The plugin does not reset provider cooldown.
The plugin does not intercept external gh work.
Prefer connected tools when their capability is needed.
Respect an explicit user choice of tools.
Installation does not change global PATH or agent settings.
New GitHub workflows are outside this change.

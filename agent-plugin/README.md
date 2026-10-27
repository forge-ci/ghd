# Agent plugin and verified tools

This plugin uses explicit version and SHA256 pins.
It never selects a latest release during a normal call.
No tool is downloaded by the gh wrapper.
The native binary still comes from the existing soldr-built wheel.
The bootstrap reads that wheel; it does not rewrite or build a wheel.
The current feature requires a wheel built from a revision containing tools publish.
A version string alone does not identify an artifact; retain its trusted SHA256.
Obtain the checksum through a trusted release/CI channel independent of the download.
The bootstrap checks the complete archive before extracting one selected member.
Native publication checks the extracted binary SHA256 again before probing it.
ZIP symlinks, duplicate members, traversal names and oversized files are rejected.
TAR publication accepts only the exact regular member.
Archive data is limited to 128 MiB; selected binaries to 64 MiB.
Install/upgrade operations are explicit and may use HTTPS or an offline archive.
Example: python agent-plugin/bootstrap.py install --kind ghd --root /absolute/private/tools --archive /path/ghd.whl --member ghd-0.1.0.data/scripts/ghd --sha256 TRUSTED_SHA256 --version 0.1.0
On Windows use the same Python entry point and the wheel member ending in ghd.exe.
Use an absolute root owned by the current user.
The native publisher applies owner-private platform permissions.
Interrupted .tool-stage-* or private-publication stages fail closed on further install.
Preserve and inspect those stages after confirming no installer is running.
Set GHD_PLUGIN_HOME to this root before using wrappers.
Runtime/status/uninstall require a separately trusted ghd publisher on PATH or GHD_PUBLISHER.
Use --ghd PATH to select that publisher explicitly.
The publisher must live outside the managed installation root.
It validates native custody before executing any managed binary.
The native delegate pins GHD_EXE so cold startup uses the verified managed build.
Optional --launchers /absolute/trusted/launchers generates the Unix gh/install.sh and CMD gh.cmd/install.cmd entry points.
Choose a new or already-owned directory outside both the managed root and repository checkout.
The directory parent must already exist.
A private bootstrap.py payload accompanies the launchers, so no mutable-root code executes.
The generated gh wrapper routes arguments through the verified ghd build.
The generated gh.cmd supplies the same route on Windows.
The wrapper waits for child completion and preserves streams/status.
Use bootstrap.py run --root ROOT -- snapshot -R owner/repo --fresh for authority.
The wrappers are not installed globally or added to PATH automatically.
Optionally add the generated launcher directory to an explicitly selected agent PATH.
The native resolver recognizes the shim marker and skips its own wrappers.
GHD_GH_PATH selects an explicit system GitHub CLI and wins over managed discovery.
To install managed gh, select its official pinned release archive and exact bin/gh member.
Example: python agent-plugin/bootstrap.py install --kind gh --root ROOT --archive GH_ARCHIVE --member EXACT_GH_MEMBER --sha256 TRUSTED_SHA256 --version PINNED_GH_VERSION --ghd /path/to/verified/ghd
GHD_MANAGED_GH_DIR selects this root for native callers.
The plugin wrapper selects a managed gh record in its own root when available.
A missing or corrupt selected record fails explicitly; no download is attempted.
Direct native API paths remain independent of gh.
status --kind gh|ghd --root ROOT --ghd VERIFIED_GHD checks installed custody/integrity.
upgrade uses the same explicit archive/version/member/checksum arguments as install.
New immutable binary objects precede atomic active-record publication.
Concurrent native publication uses a private bounded custody lock.
A busy installer fails explicitly rather than publishing concurrently.
Stop daemons and other consumers before uninstall or upgrade cleanup on Windows.
If old running binaries prevent cleanup, the operation reports an error; inspect status before retrying.
Cleanup of a prior failed upgrade must succeed before another version is published.
uninstall --kind gh|ghd --root ROOT --ghd VERIFIED_GHD removes only owned objects/record for that kind.
Lifecycle uses the separately trusted publisher, so Windows does not uninstall its running image.
Coordination directories/locks remain; other tool kinds, authentication and cache data are preserved.
Offline mode uses --archive and requires no network or release discovery.
A network failure or checksum mismatch leaves the existing active record authoritative.
HTTPS URLs must omit credentials, query strings and fragments.
The plugin manifest is under .claude-plugin; the scoped preference is under skills/ghd.
Uninstall --kind ghd --launchers DIR also removes the owned generated launchers/payload.
Use Python directly for removal if a currently running CMD installer holds its own file.
Launcher ownership markers and locks remain intentionally for safe reuse.
Tracked repository automation remains Python; no shell-policy exception is required.
Public package/catalog follow-up: https://github.com/forge-ci/ghd/issues/20
No service, hook or third-party agent setting is activated by the installer.
Public release publication and catalog distribution are separate from this repository change.
Report validation evidence in docs/GHX_PARITY.md and issue #16.

CMD entry points follow cmd.exe quoting rules. Programmatic Windows callers
can invoke the Python bootstrap directly with a raw argument list.

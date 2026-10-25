# `crates/ghd-cli/src/`

Source for the `ghd` binary (`[[bin]] name = "ghd"`, `path =
"src/ghd.rs"`). One file.

## What's here

| File | Contents |
|---|---|
| `ghd.rs` | `fn main() -> ExitCode`: split argv, dispatch, map the result to an exit code. |

## What `ghd.rs` does

```rust
let args: Vec<String> = std::env::args().skip(1).collect();
let result = match args.first().map(String::as_str) {
    Some("daemon") => ghd::run_daemon(),
    _ => ghd::cli::run(&args),
};
```

- `ghd daemon` runs the long-lived server (`ghd-daemon`, through the
  amalgam's `daemon` feature). Normally nobody types this: a client
  that finds no healthy daemon spawns `ghd daemon` detached, under a
  `start.lock` so concurrent cold clients start only one.
- Everything else goes to `ghd-core`'s `cli::run`: `pr view <n> [-R
  owner/repo] --json f1,f2`, `stats`, `--version`/`-V`,
  `--help`/`-h`/`help`, or no arguments (usage text). Unknown input
  returns an `InvalidInput` error carrying the usage text.
- `Ok(())` → `ExitCode::SUCCESS`. `Err(e)` → `ghd: <e>` on stderr and
  `ExitCode::FAILURE`.

## Why it is this small

The binary is a packaging artifact: soldr's `bundle-bins` drops it into
the wheel, and the composite action installs it. Keeping logic out of
it means the same behavior is reachable from `ghd-py` and from library
callers of the `ghd` crate, and is unit-tested once in `ghd-core` /
`ghd-daemon` instead of only through a subprocess. If you are about to
add argument parsing or output formatting here, put it in
`crates/private/ghd-core/src/cli.rs` instead.

## Why `daemon` is dispatched here, not in `cli::run`

`ghd-core` must not depend on `ghd-daemon` (the dependency direction is
`ghd-daemon → ghd-core`), and it has no async runtime or TLS. Only the
binary links both halves, so only the binary can route `daemon` to the
server.

## Dependencies

`ghd` with `features = ["daemon"]`, and `anyhow` (workspace). Nothing
else — never the private crates directly.

## Testing

`[[bin]] test = false`. Behavior is proven by `../tests/cli/main.rs`,
which runs the compiled executable:

```bash
soldr cargo test -p ghd-cli --locked
```

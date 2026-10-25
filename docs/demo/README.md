# `docs/demo/`

The source of the ghd demo: the recorded terminal session, and the scripts
that turn it into the animated GIF in the top-level README
(`docs/assets/demo.gif`) and the interactive page published on GitHub Pages
(<https://forge-ci.github.io/ghd/>, branch `gh-pages`).

Nothing here is part of the build, the wheel or CI. It is documentation
that can be regenerated.

## Files

| file | what it is |
|---|---|
| `demo.py` | the scripted session: real `ghd` and `gh` commands against `talos-hq/clud` |
| `ghd.cast` | the recording, as an asciicast v2 file (the source of truth for both outputs) |
| `to_cast.py` | `script(1)` timing + typescript -> `ghd.cast`, typing each command out |
| `to_gif.py` | `ghd.cast` -> animated GIF, drawn frame by frame with Pillow and JetBrains Mono |
| `build_page.py` | `ghd.cast` + `page.html` -> the self-contained demo page |
| `page.html` | the page template: player, chapters and the measured numbers (`__CAST__` placeholder) |

## What the session shows

1. `ghd --version`.
2. A cold read: the first call starts the daemon, which fetches over GraphQL.
3. Ten concurrent reads (five PRs, each asked twice): duplicates wait on
   one per-PR lock and are served `fresh`. Distinct PRs share batched,
   aliased GraphQL queries.
4. `ghd stats`: GraphQL points actually spent.
5. The same read through plain `gh`, for comparison.
6. The ten reads again: freshness is proven by REST ETag `304`s, which
   GitHub does not count against the rate limit (`revalidated`), so this
   round costs 0 points.
7. Two warm reads: the first revalidates (one REST round trip), the
   second is served `fresh` in a few milliseconds.
8. The Python binding reading through the same cache.

`demo.py` sets `GHD_FRESH_MS=400` so the second round falls outside the
fresh window. The default is 2 s.

## Regenerating

Recording spends a few GitHub API points and needs `gh auth` and an
installed `ghd` (`./test` installs it into `.venv`). From the repo root:

```sh
# 1. record (script(1) from util-linux)
(cd docs/demo && COLUMNS=100 LINES=40 script -q -T timing.log -O session.log -c "uv run --no-project --script demo.py")
# 2. typescript -> asciicast
uv run --no-project --script docs/demo/to_cast.py docs/demo
# 3. asciicast -> GIF (Pillow; any JetBrains Mono TTF directory)
uv run --python 3.12 --no-project --script docs/demo/to_gif.py docs/demo <font-dir> docs/assets/demo.gif
# 4. asciicast -> page (writes docs/demo/ghd-demo.html)
uv run --no-project --script docs/demo/build_page.py docs/demo
```

`timing.log`, `session.log` and `ghd-demo.html` are intermediate files; don't
commit them. To publish the page, wrap `ghd-demo.html` in a full HTML
document and push it as `index.html` on the `gh-pages` branch, together
with `demo.gif` and `.nojekyll`.

## Honesty rules

- The page and the GIF replay the recorded bytes. Only pauses longer than
  1.2 s are shortened, and each `time` line shows the real duration.
- The numbers on the page are copied from this recording's `ghd stats`
  and `time` output. Re-record, then update `page.html`'s facts to match.

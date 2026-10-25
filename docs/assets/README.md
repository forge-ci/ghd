# `docs/assets/`

Binary assets referenced from the repository's Markdown.

## Files

| file | used by | source |
|---|---|---|
| `demo.gif` | the top-level `README.md` (links to the interactive demo) | rendered from `docs/demo/ghd.cast` by `docs/demo/to_gif.py` |

## `demo.gif`

- 928×594 pixels, about 1.6 MB, 32-color palette, loops forever.
- The final screen is held for 4 s before the loop restarts.
- It shows the recorded ghd session described in `docs/demo/README.md`:
  - a cold read that starts the daemon;
  - ten concurrent reads served from cache and batched GraphQL;
  - a second round revalidated by free ETag `304`s;
  - warm-read latency, and the Python binding.
- The terminal is drawn by replaying the asciicast into a fixed
  100×30-character screen, so it matches the interactive page byte for byte
  apart from scrolling.

GitHub renders the GIF inline in the README. Clicking it opens the
interactive version on GitHub Pages: <https://forge-ci.github.io/ghd/>.

## Rules for this directory

- Keep each asset under a few MB. GitHub serves README images through its
  camo proxy, which refuses very large files, and every clone downloads
  them.
- Every asset here must be reproducible from a source that is checked in
  elsewhere (for `demo.gif`, `docs/demo/`). Don't commit one-off
  screenshots without their source.
- When an asset changes, regenerate it from its source rather than
  editing the binary, and say in the commit which source produced it.
- Reference assets with repository-relative paths
  (`docs/assets/demo.gif`), so they render on GitHub, on forks and in local
  Markdown previews.

## Regenerating `demo.gif`

```sh
uv run --python 3.12 --no-project --script docs/demo/to_gif.py \
  docs/demo <dir containing JetBrainsMono-Regular.ttf and -Bold.ttf> docs/assets/demo.gif
```

Python 3.12 is pinned because the pinned Pillow version ships no prebuilt
wheel for newer interpreters, and building it from source needs system
image libraries.

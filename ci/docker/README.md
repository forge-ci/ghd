# `ci/docker/`

Dockerfiles that back the local CI loop (zackees/ci.yml#6 section 11).
Nothing under this directory is built or referenced by
`.github/workflows/ci.yml` itself -- GitHub-hosted runners never touch
`ci/docker/**`. Everything here exists so `python3 ci/local.py act` can
run the same workflow locally through bosn -> act.

## Layout

```
ci/docker/
└── act/
    ├── Dockerfile.bosn-act   # the bosn "act" stack image: pinned act + docker CLI + python3
    ├── Dockerfile.runner     # the act job-container image: catthehacker/ubuntu:act-24.04 + clang
    └── README.md             # what each image is for, and how they fit together
```

## Why two separate images

They play different roles in the act model:

- **`Dockerfile.bosn-act`** builds the container bosn itself manages
  (`bosn.toml`'s `[stack.act]`). It carries the `act` binary and a
  Docker CLI, and drives the *host* Docker engine through a bind-mounted
  `/var/run/docker.sock`. It never runs a workflow job itself.
- **`Dockerfile.runner`** builds the image act uses *for job containers*
  (`-P ubuntu-24.04=<tag>`, replacing the un-pinned
  `catthehacker/ubuntu:act-24.04` default). These are the sibling
  containers act creates on the host engine for each `runs-on:
  ubuntu-24.04` job.

Conflating them would mean every job container also carries the `act`
binary and Docker CLI it doesn't need, and every act-driving container
would carry `clang` it doesn't need either.

## No shell scripts

This repository's fleet policy (`AGENTS.md`, `ci.toml` precheck group 3)
bans checked-in `.sh`/`.ps1`/`.bat`/`.cmd` files anywhere in the repo --
a `RUN` line's inline shell *inside* a Dockerfile is a normal Dockerfile
construct and is not affected, but a **checked-in wrapper script that
`bosn.toml` shells out to** is exactly what zccache's and clud's
equivalent stacks use (`run_act_test_action.sh`, `act_ci.sh`) and exactly
what this repository cannot replicate. `bosn.toml`'s `[task.act-run]`
therefore runs `python3 /work/ci/local.py act-inner` directly --
see `ci/localrun/act_inner.py` for the logic those shell scripts would
otherwise have held.

## Pinning

Both Dockerfiles pin their base image by registry digest
(`FROM <image>@sha256:...`), and `Dockerfile.bosn-act` additionally
verifies the `act` and Docker-CLI tarballs it downloads against a
pinned sha256 (independently re-verified against the upstream release
assets on 2026-09-28, matching zackees/zccache's own pins for the same
`act` version). See each Dockerfile's own comments for exactly what is
and is not fully reproducible (`clang` itself comes from Ubuntu 24.04's
apt repositories, which do not offer a tarball-style sha256 pin -- the
base image's digest is the strong guarantee there).

## Building

Nothing here is built manually. `bosn ensure --stack act` (or the first
`bosn run --task act-run`) builds `Dockerfile.bosn-act` through bosn's
own managed-image lifecycle. `ci/localrun/act_inner.py` builds
`Dockerfile.runner` itself, on demand, skipping the rebuild once the
tag is already present (`docker image inspect`) -- see
`ci/docker/act/README.md` for the exact tag name and rebuild triggers.

"""Perf-lane orchestration (zackees/ci.yml#6 round 5, deliverable 4):
`ci.toml [suites].perf = { run = "ci/perf.py", kind = "bench", gating =
false }`. Each `.github/workflows/ci.yml` `perf` job step is one line
calling `python3 ci/perf.py <subcommand>` (CLAUDE.md rule 6 / `GEN-005`);
this module holds the logic, mirroring every other `ci/*.py`
orchestration script in this repo (self-contained, stdlib-only, never
imports another `ci/*.py` module).

AGENTS.md's typed-benchmark rule applies directly: benchmark samples and
reports are frozen dataclasses; the JSON file `ci-lint perf compare`
reads is a wire boundary only, filled by `_write_benchmark_file` and
never re-read as a raw dict inside this process. A boundary dict from
the GitHub REST API (`_ArtifactRecord`) is likewise converted to a
dataclass immediately after parsing, before any comparison/selection
logic runs.

Two subcommands:
  - `bench`: builds the release-profile linux-x64 wheel (same shape as
    `ci/release.py build`, duplicated rather than imported -- see that
    module's docstring for why), installs it into a clean venv, then
    times (a) `ghd-cli --version` process startup xN and (b) one
    PyO3 call (`ghd.bindings.version_banner()`) xM,
    the latter via `ci/perf_pyo3_bench.py` run BY that venv's own python
    (so the timed import is the installed extension, not this
    checkout's editable `src/` tree). Writes a `BenchmarkFile` JSON.
  - `fetch-baseline`: GitHub REST API (stdlib `urllib`, the job's own
    `GITHUB_TOKEN`, `actions: read` -- never printed) lookup of the
    latest artifact named `perf-results` from a SUCCESSFUL run on
    `--branch` (default `main`; covers both the `main`-push writer flow
    and the nightly schedule, which also targets `main`). When none
    exists yet, writes a valid, empty `BenchmarkFile` and reports "no
    baseline" -- this subcommand always exits 0 (issue #6 §5/round 5:
    "if none exists yet, the job reports 'no baseline' and still
    succeeds").
"""

from __future__ import annotations

import argparse
import io
import json
import os
import statistics
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BENCHMARK_SCHEMA_VERSION = 1
API_TIMEOUT_SECONDS = 15


def _run_isolated_soldr(cmd: list[str]) -> int:
    """See `ci/fast.py::_run_isolated_soldr` -- identical reasoning."""
    env = {
        k: v for k, v in os.environ.items() if not k.startswith(("SOLDR_", "ZCCACHE_"))
    }
    print(
        f"+ {' '.join(cmd)}  (SOLDR_*/ZCCACHE_* stripped -- see docstring)", flush=True
    )
    return subprocess.run(cmd, cwd=ROOT, env=env, check=False).returncode


# ---------------------------------------------------------------------------
# Typed benchmark model (AGENTS.md: dataclasses, never raw dicts; JSON
# only at the I/O boundary). Field names/shape match `ci_lint.perf`'s
# `BenchmarkRecord`/`BenchmarkFile` exactly -- this module writes the
# wire format `ci-lint perf compare` reads, it does not import that
# package's dataclasses (ci/*.py treats `ci_lint` as an external pinned
# CLI tool, never a library -- see `ci/platform_build.py`'s docstring).
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class BenchmarkRecord:
    name: str
    unit: str
    samples: tuple[float, ...]
    median: float

    def to_json_dict(self) -> dict[str, object]:
        return {
            "name": self.name,
            "unit": self.unit,
            "samples": list(self.samples),
            "median": self.median,
        }


@dataclass(frozen=True)
class BenchmarkFile:
    schema_version: int
    benchmarks: tuple[BenchmarkRecord, ...]

    def to_json_dict(self) -> dict[str, object]:
        return {
            "schema_version": self.schema_version,
            "benchmarks": [b.to_json_dict() for b in self.benchmarks],
        }


def _record(name: str, unit: str, samples: list[float]) -> BenchmarkRecord:
    if not samples:
        raise ValueError(f"benchmark {name!r} produced zero samples")
    return BenchmarkRecord(
        name=name, unit=unit, samples=tuple(samples), median=statistics.median(samples)
    )


def _write_benchmark_file(path: Path, records: list[BenchmarkRecord]) -> None:
    bench_file = BenchmarkFile(
        schema_version=BENCHMARK_SCHEMA_VERSION, benchmarks=tuple(records)
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(bench_file.to_json_dict(), indent=2), encoding="utf-8")


# ---------------------------------------------------------------------------
# bench
# ---------------------------------------------------------------------------


def _time_cli_startup(cli_path: Path, iterations: int) -> list[float]:
    samples: list[float] = []
    for _ in range(iterations):
        start = time.perf_counter()
        proc = subprocess.run(
            [str(cli_path), "--version"], cwd=ROOT, check=False, capture_output=True
        )
        elapsed = time.perf_counter() - start
        if proc.returncode != 0:
            raise RuntimeError(f"{cli_path} --version exited {proc.returncode}")
        samples.append(elapsed)
    return samples


def _time_pyo3_calls(venv_python: Path, iterations: int) -> list[float]:
    proc = subprocess.run(
        [
            str(venv_python),
            str(ROOT / "ci" / "perf_pyo3_bench.py"),
            "--iterations",
            str(iterations),
        ],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise RuntimeError(
            f"perf_pyo3_bench.py exited {proc.returncode}: {proc.stderr.strip()}"
        )
    samples = json.loads(proc.stdout.strip().splitlines()[-1])
    if not isinstance(samples, list) or not samples:
        raise RuntimeError(f"perf_pyo3_bench.py produced no samples: {proc.stdout!r}")
    return [float(s) for s in samples]


def cmd_bench(args: argparse.Namespace) -> int:
    wheel_dir = Path(args.wheel_dir)
    wheel_dir.mkdir(parents=True, exist_ok=True)
    rc = _run_isolated_soldr(
        [
            "uv",
            "build",
            "--wheel",
            "--config-setting",
            "profile=release",
            "--out-dir",
            str(wheel_dir),
        ]
    )
    if rc != 0:
        return rc
    wheels = sorted(wheel_dir.glob("*.whl"))
    if not wheels:
        print(f"ci/perf.py: uv build produced no .whl in {wheel_dir}", file=sys.stderr)
        return 1
    wheel_path = wheels[-1]

    venv_dir = Path(args.venv)
    rc = subprocess.run(
        ["uv", "venv", str(venv_dir), "--python", args.python], cwd=ROOT, check=False
    ).returncode
    if rc != 0:
        return rc
    venv_python = venv_dir / "bin" / "python3"  # this job is ubuntu-24.04-only
    rc = subprocess.run(
        ["uv", "pip", "install", "--python", str(venv_python), str(wheel_path)],
        cwd=ROOT,
        check=False,
    ).returncode
    if rc != 0:
        return rc

    cli_path = venv_dir / "bin" / args.cli_name
    if not cli_path.is_file():
        print(f"ci/perf.py: installed wheel has no {cli_path}", file=sys.stderr)
        return 1

    try:
        cli_samples = _time_cli_startup(cli_path, args.cli_iterations)
        pyo3_samples = _time_pyo3_calls(venv_python, args.pyo3_iterations)
    except (RuntimeError, json.JSONDecodeError) as exc:
        print(f"ci/perf.py: {exc}", file=sys.stderr)
        return 1

    records = [
        _record(f"{args.cli_name}_startup", "s", cli_samples),
        _record("pyo3_version_banner_call", "s", pyo3_samples),
    ]
    out_path = Path(args.out)
    _write_benchmark_file(out_path, records)
    for r in records:
        print(f"{r.name}: median={r.median * 1000:.3f}ms over {len(r.samples)} samples")
    print(f"wrote {out_path}")
    return 0


# ---------------------------------------------------------------------------
# fetch-baseline
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class _ArtifactRecord:
    """Typed slice of one GitHub Actions artifact list-item -- the only
    fields this module reads, converted immediately out of the raw JSON
    dict (AGENTS.md: never pass a raw dict past the function that parses
    the wire boundary)."""

    artifact_id: int
    name: str
    created_at: str
    expired: bool
    workflow_run_id: int | None
    head_branch: str | None


def _api_get(url: str, token: str) -> dict[str, object]:
    req = urllib.request.Request(
        url,
        headers={
            "Authorization": f"Bearer {token}",
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    with urllib.request.urlopen(req, timeout=API_TIMEOUT_SECONDS) as resp:  # noqa: S310 (github API only)
        body = json.loads(resp.read().decode("utf-8"))
    if not isinstance(body, dict):
        raise RuntimeError(
            f"GET {url}: expected a JSON object, got {type(body).__name__}"
        )
    return body


def _list_artifacts(repo: str, token: str, artifact_name: str) -> list[_ArtifactRecord]:
    records: list[_ArtifactRecord] = []
    page = 1
    while (
        page <= 5
    ):  # up to 500 artifacts -- this repo's history is small; bounded regardless
        url = (
            f"https://api.github.com/repos/{repo}/actions/artifacts"
            f"?name={urllib.parse.quote(artifact_name)}&per_page=100&page={page}"
        )
        body = _api_get(url, token)
        raw_artifacts = body.get("artifacts")
        if not isinstance(raw_artifacts, list) or not raw_artifacts:
            break
        for raw in raw_artifacts:
            if not isinstance(raw, dict):
                continue
            workflow_run = raw.get("workflow_run")
            run_id = None
            head_branch = None
            if isinstance(workflow_run, dict):
                raw_run_id = workflow_run.get("id")
                run_id = raw_run_id if isinstance(raw_run_id, int) else None
                raw_branch = workflow_run.get("head_branch")
                head_branch = raw_branch if isinstance(raw_branch, str) else None
            artifact_id = raw.get("id")
            created_at = raw.get("created_at")
            expired = raw.get("expired")
            if not isinstance(artifact_id, int) or not isinstance(created_at, str):
                continue
            records.append(
                _ArtifactRecord(
                    artifact_id=artifact_id,
                    name=str(raw.get("name", "")),
                    created_at=created_at,
                    expired=bool(expired),
                    workflow_run_id=run_id,
                    head_branch=head_branch,
                )
            )
        if len(raw_artifacts) < 100:
            break
        page += 1
    return records


def _run_succeeded(repo: str, token: str, run_id: int) -> bool:
    body = _api_get(f"https://api.github.com/repos/{repo}/actions/runs/{run_id}", token)
    return body.get("conclusion") == "success"


def _download_artifact_json(
    repo: str, token: str, artifact_id: int, out_path: Path
) -> bool:
    """Download+unzip the artifact, copying its one `*.json` member to
    `out_path`. Returns False (never raises) if the archive has no JSON
    member -- treated the same as "no usable baseline"."""
    url = f"https://api.github.com/repos/{repo}/actions/artifacts/{artifact_id}/zip"
    req = urllib.request.Request(
        url,
        headers={
            "Authorization": f"Bearer {token}",
            "Accept": "application/vnd.github+json",
        },
    )
    with urllib.request.urlopen(req, timeout=API_TIMEOUT_SECONDS) as resp:  # noqa: S310
        payload = resp.read()
    with zipfile.ZipFile(io.BytesIO(payload)) as zf:
        json_members = [n for n in zf.namelist() if n.endswith(".json")]
        if not json_members:
            return False
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_bytes(zf.read(json_members[0]))
    return True


def cmd_fetch_baseline(args: argparse.Namespace) -> int:
    out_path = Path(args.out)
    token = os.environ.get("GITHUB_TOKEN", "")
    if not token:
        print("ci/perf.py: fetch-baseline: no baseline (GITHUB_TOKEN not set)")
        _write_benchmark_file(out_path, [])
        return 0

    try:
        candidates = _list_artifacts(args.repo, token, args.artifact_name)
    except (urllib.error.URLError, RuntimeError, json.JSONDecodeError) as exc:
        print(
            f"ci/perf.py: fetch-baseline: no baseline (artifact list lookup failed: {exc})"
        )
        _write_benchmark_file(out_path, [])
        return 0

    on_branch = [
        c
        for c in candidates
        if not c.expired
        and c.head_branch == args.branch
        and c.workflow_run_id is not None
    ]
    on_branch.sort(key=lambda c: c.created_at, reverse=True)

    for candidate in on_branch:
        try:
            if not _run_succeeded(args.repo, token, candidate.workflow_run_id):  # type: ignore[arg-type]
                continue
            if _download_artifact_json(
                args.repo, token, candidate.artifact_id, out_path
            ):
                print(
                    f"ci/perf.py: fetch-baseline: using artifact {candidate.artifact_id} "
                    f"from run {candidate.workflow_run_id} (created {candidate.created_at})"
                )
                return 0
        except (urllib.error.URLError, RuntimeError) as exc:
            print(
                f"ci/perf.py: fetch-baseline: candidate {candidate.artifact_id} failed ({exc}), trying next"
            )
            continue

    print(
        f"ci/perf.py: fetch-baseline: no baseline (no successful '{args.artifact_name}' "
        f"artifact found on branch {args.branch!r} yet)"
    )
    _write_benchmark_file(out_path, [])
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    p = sub.add_parser("bench")
    p.add_argument("--wheel-dir", required=True)
    p.add_argument("--venv", required=True)
    p.add_argument("--python", default="3.11")
    p.add_argument("--cli-name", default="ghd")
    p.add_argument("--cli-iterations", type=int, default=30)
    p.add_argument("--pyo3-iterations", type=int, default=2000)
    p.add_argument("--out", required=True)
    p.set_defaults(func=cmd_bench)

    p = sub.add_parser("fetch-baseline")
    p.add_argument("--repo", required=True, help="owner/repo")
    p.add_argument("--artifact-name", default="perf-results")
    p.add_argument("--branch", default="main")
    p.add_argument("--out", required=True)
    p.set_defaults(func=cmd_fetch_baseline)

    args = parser.parse_args(argv)
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main())

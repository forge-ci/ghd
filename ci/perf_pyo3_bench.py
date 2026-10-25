"""Inner PyO3-call benchmark loop, executed by the release-profile venv's
OWN python (`<venv>/bin/python3 ci/perf_pyo3_bench.py --iterations N`) --
NOT the outer system `python3` `ci/perf.py` itself runs under. Running
inside that venv is what makes the timed `import` resolve the actually
INSTALLED, compiled `_native` extension (the same wheel `ci/perf.py
bench` just built and installed), never this checkout's editable `src/`
tree.

zackees/ci.yml#6 round 5 deliverable 4: "a small, real benchmark of the
product ... a PyO3 call xN". Prints exactly ONE line of JSON to stdout
(a bare list of per-call second counts) -- `ci/perf.py`'s `bench`
subcommand is the only consumer and does all `BenchmarkRecord`/
dataclass assembly itself (AGENTS.md's typed-benchmark rule); this
script never touches a dataclass or writes a file, so it stays a
minimal, dependency-free timing loop.
"""

from __future__ import annotations

import argparse
import json
import time

from ghd.bindings import version_banner


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iterations", type=int, required=True)
    args = parser.parse_args()

    # One untimed warm-up call: the module is already imported by the
    # time argparse finishes, but this keeps any first-call-only cost
    # (e.g. a lazily-initialized static inside the extension) out of the
    # very first timed sample.
    version_banner()

    samples: list[float] = []
    for _ in range(args.iterations):
        start = time.perf_counter()
        version_banner()
        samples.append(time.perf_counter() - start)

    print(json.dumps(samples))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

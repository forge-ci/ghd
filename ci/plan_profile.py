"""Derive whether this run's resolved flow wants RELEASE-profile builds
(zackees/ci.yml#6 round 5, deliverable 2). `ci.toml`'s `[flow.release]`
and `[flow.nightly]` are the only two flows that build the full wheel
matrix for every platform ("full sweep" / "release rehearsal"); `pr` and
`main` stay dev profile (unchanged from round 3/4).

One line in `ci-pre.yml`'s `precheck` job, reading `ci_lint plan`'s
own `plan.flow` field -- this never re-derives flow-selection logic
itself; `ci_lint` is the one source of truth for what flow a run
resolved to. The single `is-release` boolean output is then reused by
every release-profile-aware step in `ci.yml` (`release-guard`,
`release-linux-x64`, `platform-build`'s/`platform-run`'s profile
selection) instead of repeating the `fromJSON(...).flow == 'release' ||
... == 'nightly'` expression at each call site.
"""

from __future__ import annotations

import json
import os
import sys

# The only two flows that build release-profile wheels for every declared
# platform (ci.toml [flow.release] / [flow.nightly], the latter `extends
# = "release"`). `pr`/`main` never do, even when a PR carries `[release]`
# rehearsal's sibling tags -- `[tags].release` itself switches the whole
# run onto flow "release", so this set only ever needs these two names.
RELEASE_FLOWS = frozenset({"release", "nightly"})


def main() -> int:
    raw = os.environ.get("PLAN_JSON", "")
    if not raw:
        print("ci/plan_profile.py: PLAN_JSON is empty", file=sys.stderr)
        return 1
    try:
        plan = json.loads(raw)
    except json.JSONDecodeError as exc:
        print(
            f"ci/plan_profile.py: PLAN_JSON is not valid JSON: {exc}", file=sys.stderr
        )
        return 1
    if not isinstance(plan, dict):
        print("ci/plan_profile.py: PLAN_JSON must be a JSON object", file=sys.stderr)
        return 1

    flow = plan.get("flow")
    is_release = flow in RELEASE_FLOWS
    profile = "release" if is_release else "dev"
    print(
        f"ci/plan_profile.py: plan.flow={flow!r} -> is-release={is_release}, profile={profile!r}"
    )

    gh_out = os.environ.get("GITHUB_OUTPUT")
    if gh_out:
        with open(gh_out, "a", encoding="utf-8") as fh:
            fh.write(f"is-release={'true' if is_release else 'false'}\n")
            # A plain "release"/"dev" string, NOT the `is-release == 'true'
            # && 'release' || 'dev'` ternary this replaces: GEN-005's
            # shell-budget scan flags `&&`/`||` textually inside a `run:`
            # line's own `${{ }}` expression, even though the Actions
            # runtime substitutes it to a plain string before the shell
            # ever sees it (found empirically -- `ci_lint precheck` on
            # this round's first draft). `--profile
            # ${{ needs.precheck.outputs.profile }}` has no such token.
            fh.write(f"profile={profile}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

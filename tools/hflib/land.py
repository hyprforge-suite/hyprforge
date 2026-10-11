"""`hf land <component> [--next]`: a component's work, pushed in the
order CLAUDE.md gives, then its pin staged here.

A change to a component is two commits and the order is the rule: the
component's commit on its repository first, the pin naming it second.
A pin on a commit nobody can fetch is a monorepo nobody else can clone.

`--next` is for work that needs a library change not yet on crates.io.
It builds here, through [patch.crates-io], but would fail the
component's own CI, so it waits on the component's `next` branch and
no pin names it; `hf release` merges every `next` once the libraries it
needs are published. That replaces the hand-split commits and
`after-0.1.14` branches the 0.1.13 release needed.
"""

from . import publishing, ui
from .run import ROOT, run


def land(component: str, to_next: bool) -> int:
    path = publishing.component_path(component)
    name = path.split("/")[-1]

    ui.step(f"Landing {name}" + (" on next" if to_next else ""))
    dirty = run(["git", "-C", path, "status", "--porcelain"]).out.strip()
    if dirty:
        ui.bad(f"{path} has uncommitted changes — commit inside it first, staging only the paths you wrote "
               f"(git -C {path} add <paths>, never -A)")
        return ui.summary()
    branch = run(["git", "-C", path, "branch", "--show-current"]).out.strip() or "(detached)"
    head = run(["git", "-C", path, "rev-parse", "--short", "HEAD"]).out.strip()
    run(["git", "-C", path, "fetch", "-q", "origin"], timeout=120)

    if to_next:
        r = run(["git", "-C", path, "push", "origin", "HEAD:next"], timeout=120)
        if not r.ok:
            ui.bad(f"push to {name}'s next failed")
            ui.detail(r.lines()[-5:])
            return ui.summary()
        ui.ok(f"{name} {head} is on its next branch; no pin staged — the release merges it")
        return ui.summary()

    if not run(["git", "-C", path, "merge-base", "--is-ancestor", "origin/main", "HEAD"]).ok:
        ui.bad(f"{name} {head} (on {branch}) does not descend from origin/main — rebase or merge it first")
        return ui.summary()

    ui.note("building it alone against crates.io, the way its own CI is about to …")
    built, errors = publishing.standalone_build(path)
    if not built:
        ui.bad(f"{name} does not build on its own against the published libraries")
        ui.detail(errors.splitlines())
        ui.note(f"if it needs an unreleased library change, land it on next instead: tools/hf land {name} --next")
        return ui.summary()
    ui.ok("builds against the published libraries")

    r = run(["git", "-C", path, "push", "origin", "HEAD:main"], timeout=120)
    if not r.ok:
        ui.bad(f"push to {name}'s main failed")
        ui.detail(r.lines()[-5:])
        return ui.summary()
    ui.ok(f"{name} {head} pushed to its main")

    run(["git", "add", path], cwd=ROOT)
    ui.ok(f"pin staged here: git commit when ready (git diff --cached {path})")
    return ui.summary()


def precommit() -> int:
    """What the pre-commit hook asks before the fast check: that no
    staged pin names a commit missing from its repository's main."""
    unpushed = publishing.staged_pins_not_on_main()
    if unpushed:
        ui.step("Staged pins are on their repositories")
        for path, sha in unpushed:
            ui.bad(f"{path} is pinned at {sha}, which is not on origin/main — push it first: tools/hf land {path.split('/')[-1]}")
        return ui.summary()
    return 0

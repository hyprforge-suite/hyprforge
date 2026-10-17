"""The pins build together: the tree CI builds, built here.

CI checks out every component at its pinned commit. This checkout often
does not: a component on its `next` branch builds here against the
library change it was written for, and every local check passes. When
that library change is one the *pinned* component cannot compile
against, CI on main fails from that commit on, and nothing here says so.
738b5ab did exactly that — `Ask::Unlock` in files-core, matched
exhaustively by the pinned Files — and main stayed red for four days.

So when any component is checked out away from its pin, this builds the
tree CI would: a worktree of HEAD, with this checkout's uncommitted
changes applied (a pre-commit run is checking what is about to be
committed, not HEAD), and each component a worktree of its own
repository at its pinned commit. Everything comes from local objects;
nothing is fetched. The tree sits at one fixed path under target/, so
cargo's incremental state carries from one run to the next.

`cargo check`, not clippy: the question is whether it compiles, and
clippy over the workspace has already run on this checkout.
"""

import shutil
import subprocess
from pathlib import Path

from .run import BUILD, ROOT, cargo_env, run

# A git hook runs with these set for the repository it was called in —
# GIT_INDEX_FILE as a relative `.git/index` — and a `git worktree add`
# that inherits them looks for the new tree's index in the wrong place
# ("index file open failed: Not a directory"). The pre-commit hook is
# where this step matters most, so every git command here runs without
# them.
HOOK_ENV = {k: None for k in ("GIT_INDEX_FILE", "GIT_DIR", "GIT_WORK_TREE", "GIT_PREFIX", "GIT_OBJECT_DIRECTORY")}


def _git(*args: str, cwd=None):
    return run(["git", *args], cwd=cwd, env=HOOK_ENV)


def _plain_env() -> dict:
    import os

    return {k: v for k, v in os.environ.items() if k not in HOOK_ENV}


TREE = ROOT / "target" / "hf" / "pins" / "tree"
TARGET = ROOT / "target" / "hf" / "pins" / "target"


def submodules() -> list[tuple[str, str]]:
    """(path, pinned commit) for every component."""
    out = []
    for line in _git("ls-files", "-s").lines():
        mode, sha, _stage, path = line.split(maxsplit=3)
        if mode == "160000":
            out.append((path, sha))
    return out


def moved() -> list[str]:
    """Components checked out at anything but their pin."""
    return [l.split()[1] for l in _git("submodule", "status").lines() if l[:1] in "+U"]


def _remove_tree() -> None:
    # Each component's worktree is registered in its own repository;
    # removing the directory alone would leave a stale registration that
    # refuses the next `worktree add` at the same path.
    for path, _ in submodules():
        _git("-C", str(ROOT / path), "worktree", "remove", "--force", str(TREE / path))
        _git("-C", str(ROOT / path), "worktree", "prune")
    _git("worktree", "remove", "--force", str(TREE))
    _git("worktree", "prune")
    if TREE.exists():
        shutil.rmtree(TREE)


def build(features: list[str]) -> tuple[bool, list[str]]:
    """Build the pinned tree. (built, what to say)."""
    _remove_tree()
    TREE.parent.mkdir(parents=True, exist_ok=True)
    try:
        r = _git("worktree", "add", "--detach", "-q", str(TREE), "HEAD")
        if not r.ok:
            return False, [f"could not make the worktree: {r.out.strip()}"]
        # The changes about to be committed, minus the components: their
        # pins are what this tree is about.
        paths = [p for p, _ in submodules()]
        diff = subprocess.run(
            ["git", "diff", "HEAD", "--binary", "--", ".", *[f":(exclude){p}" for p in paths]],
            cwd=ROOT, capture_output=True, timeout=60, env=_plain_env(),
        ).stdout
        if diff:
            applied = subprocess.run(["git", "apply", "--whitespace=nowarn"], cwd=TREE, input=diff, capture_output=True, timeout=60, env=_plain_env())
            if applied.returncode != 0:
                return False, ["this checkout's uncommitted changes did not apply to HEAD's tree:", applied.stderr.decode(errors="replace").strip()]
        for path, sha in submodules():
            dest = TREE / path
            if dest.exists():
                dest.rmdir()  # the empty directory a submodule leaves in a worktree
            r = _git("-C", str(ROOT / path), "worktree", "add", "--detach", "-q", str(dest), sha)
            if not r.ok:
                return False, [f"{path}: its pinned commit {sha[:7]} is not in the local repository: {r.out.strip()}"]
        r = run(
            ["cargo", "check", "--workspace", "--all-targets", "--offline", "--message-format", "short", *features],
            cwd=TREE, env={**cargo_env(), **HOOK_ENV, "CARGO_TARGET_DIR": str(TARGET)}, timeout=BUILD,
        )
        if r.ok:
            return True, []
        # `--message-format short` puts the file first: `src/x.rs:3:5:
        # error[E0004]: …`, then cargo's own `error: could not compile`.
        errors = [l for l in r.out.splitlines() if l.startswith("error") or ": error" in l]
        return False, errors or r.out.splitlines()[-20:]
    finally:
        _remove_tree()

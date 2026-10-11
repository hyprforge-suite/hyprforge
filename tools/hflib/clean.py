"""`hf clean` and `hf doctor`: the machine the checks run on.

`target/` reached 439GB once and 666GB once, each time stalling the
whole computer (disk-full-build-stall). Neither time did free space
warn — the second time 98GB was still free. So `clean` reports what
build output weighs and removes only what is always safe to: the
throwaway trees release checks build into, and incremental state, which
cargo rebuilds on demand.
"""

import shutil
from pathlib import Path

from . import ui
from .run import ROOT, have, run

ALWAYS_SAFE = ["target/standalone", "target/package", "target/sync-standalone", "target/makepkg"]
INCREMENTAL = ["target/debug/incremental", "target/release/incremental"]


def _size_gb(path: Path) -> int:
    if not path.exists():
        return 0
    r = run(["du", "-s", "--block-size=1G", str(path)], timeout=300)
    try:
        return int(r.out.split()[0])
    except (IndexError, ValueError):
        return 0


def clean(dry_run: bool, incremental: bool) -> int:
    ui.step("Build output")
    targets = [ROOT / "target", *sorted((ROOT / ".claude" / "worktrees").glob("*/target"))]
    for t in targets:
        if t.exists():
            print(f"  {_size_gb(t):>5} GB  {t.relative_to(ROOT)}")
    doomed = [ROOT / p for p in ALWAYS_SAFE]
    if incremental:
        doomed += [ROOT / p for p in INCREMENTAL]
    ui.step("Removing" if not dry_run else "Would remove (dry run)")
    for p in doomed:
        if p.exists():
            size = _size_gb(p)
            print(f"  {size:>5} GB  {p.relative_to(ROOT)}")
            if not dry_run:
                shutil.rmtree(p)
    if not incremental:
        ui.note("incremental state kept; --incremental removes it too (cargo rebuilds it on demand)")
    ui.note("btrfs: if df does not move, a snapshot still holds the blocks — `sudo snapper -c home list`")
    return 0


def doctor() -> int:
    ui.step("Tools")
    for program, why in [
        ("cargo-nextest", "runs every tier — sudo pacman -S cargo-nextest"),
        ("clang", "builds procman's eBPF counter (netbpf)"),
        ("gh", "GitHub releases and watching publish runs"),
        ("makepkg", "renders the AUR .SRCINFO"),
        ("extra-x86_64-build", "the clean-chroot AUR build (devtools)"),
        ("start-hyprland", "the disposable nested Hyprland"),
        ("hyprpaper", "the disposable tier's wallpaper tests"),
        ("hyprsunset", "the disposable tier's temperature tests"),
        ("wl-copy", "the nested clipboard tests (wl-clipboard)"),
    ]:
        if have(program):
            ui.ok(program)
        else:
            ui.skip(program, why)
    ui.step("Repository")
    hooks = run(["git", "config", "core.hooksPath"]).out.strip()
    if hooks == ".githooks":
        ui.ok("pre-commit hook active (core.hooksPath = .githooks)")
    else:
        ui.bad("pre-commit hook not active — run: git config core.hooksPath .githooks")
    empty = [l.split()[1] for l in run(["git", "submodule", "status"]).lines() if l.startswith("-")]
    if empty:
        ui.bad(f"{len(empty)} submodule(s) not checked out — run: git submodule update --init")
    else:
        ui.ok("every component submodule is checked out")
    return ui.summary()

"""Proof that a check left the session the way it found it.

On 2026-10-10 the live tests pushed every option into the session in
use, and nothing noticed until the desktop froze. The tests are fixed
(hyprforge-testkit refuses the session), but a fix is not an instrument:
this snapshots everything a test could plausibly leave behind before the
live tiers and again after, and fails naming what moved.

What is compared:
  - every config option's current value (`hyprctl descriptions -j`:
    one call, ~350 options, each with `current`);
  - binds, workspace rules and animations;
  - the monitors' modes, scales and transforms (not positions of
    windows, which the person at the keyboard moves all the time);
  - hyprpaper's active wallpapers and hyprsunset's temperature.

A person changing a setting while the check runs will trip it, and the
message says that is the innocent explanation.
"""

import json

from .run import have, run


def _json(args: list[str]):
    r = run(["hyprctl", *args, "-j"], timeout=30)
    if not r.ok:
        return {"error": r.out.strip()[:200]}
    try:
        return json.loads(r.out)
    except json.JSONDecodeError:
        return {"unparsed": r.out.strip()[:200]}


def snapshot() -> dict | None:
    """None when there is no Hyprland session to guard."""
    if not have("hyprctl") or not run(["hyprctl", "version"]).ok:
        return None
    snap: dict = {}
    options = _json(["descriptions"])
    for d in options if isinstance(options, list) else []:
        snap[f"option {d.get('name')}"] = d.get("current")
    snap["binds"] = sorted(
        json.dumps({k: b.get(k) for k in ("modmask", "key", "dispatcher", "arg", "description")}, sort_keys=True)
        for b in (_json(["binds"]) or [])
        if isinstance(b, dict)
    )
    snap["workspacerules"] = json.dumps(_json(["workspacerules"]), sort_keys=True)
    snap["animations"] = json.dumps(_json(["animations"]), sort_keys=True)
    for m in _json(["monitors"]) or []:
        if isinstance(m, dict):
            snap[f"monitor {m.get('name')}"] = {
                k: m.get(k) for k in ("width", "height", "refreshRate", "scale", "transform", "vrr", "dpmsStatus")
            }
    paper = run(["hyprctl", "hyprpaper", "listactive"], timeout=10)
    if paper.ok:
        snap["hyprpaper"] = sorted(paper.lines())
    sunset = run(["hyprctl", "hyprsunset", "temperature"], timeout=10)
    if sunset.ok:
        snap["hyprsunset"] = sunset.out.strip()
    return snap


def diff(before: dict, after: dict) -> list[str]:
    changed = []
    for key in sorted(set(before) | set(after)):
        if before.get(key) != after.get(key):
            if key.startswith("option ") or key.startswith("monitor ") or key in ("hyprsunset",):
                changed.append(f"{key}: {before.get(key)!r} -> {after.get(key)!r}")
            else:
                changed.append(f"{key} changed")
    return changed

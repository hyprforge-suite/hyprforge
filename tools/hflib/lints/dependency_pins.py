#!/usr/bin/env python3
# Standalone crate dependency pins.
#
# Moved verbatim from check.sh, where it ran as an inline heredoc;
# `tools/hf lint` runs it from the repository root and reads its
# CHECKED / PROBLEM / SKIP lines.
#
# hyprforge-clipboard, hyprforge-lock and hyprforge-greet are prepared to
# become their own repositories (see docs/design/repo-plan.md), which means each of
# them now spells out every third-party dependency's version and feature
# set in full — a crate that is its own repository root has no
# `[workspace.dependencies]` to inherit from, so `serde.workspace = true`
# becomes `serde = { version = "1", features = ["derive"] }`, copied by
# hand.
#
# That is required, and it is a silent drift hazard: if the root table
# bumps a version or a feature and one of these three manifests is not
# updated to match, cargo resolves both happily. Clippy stays silent,
# every test stays green, and the suite quietly builds two different
# versions of the same dependency — the "one place to fix a shared
# thing" property (see CLAUDE.md and docs/design/repo-plan.md) failing with no
# symptom at all. Nothing else here would ever notice, because nothing
# else compares a standalone crate's manifest against the root one.
#
# Which crates to check is discovered, not hardcoded: a crate counts as
# standalone-ready when nothing in its manifest inherits from the
# workspace at all — no `version.workspace = true`, no
# `foo.workspace = true` dependency. That is deliberately stronger than
# "has its own version number": hyprforge-paths, hyprforge-look,
# hyprforge-secret, hyprforge-process and hyprforge-ui also carry their
# own version (docs/design/repo-plan.md step 1, for publishing to crates.io) while
# every one of their dependencies is still `.workspace = true` — they
# are publish-ready, not standalone-ready, and have nothing to drift.
# Discovering the list this way means the next crate someone prepares
# for a split is covered automatically, instead of silently not being
# covered — which is the exact failure this project keeps naming.
#
# Only dependencies present in BOTH the crate and the root table are
# compared. A crate-exclusive dependency (`xkbcommon`,
# `wayland-protocols`) has no root value to drift from. And
# `hyprforge-paths`, `hyprforge-look`, `hyprforge-authui` and
# `hyprforge-secret` are exempt by name: those four are deliberately
# spelled as a git dependency in the standalone crate and a local path in
# the root table — that mismatch in *form* is the split mechanism
# working (see the `[patch]` section in Cargo.toml), not drift, and
# there is no version there to compare in the first place.
#
# A dropped feature is flagged; an added one is not. Extending a shared
# dependency with extra features on top of the workspace base is the
# normal pattern here (`hyprforge-settings` adds `tokio` to `iced`,
# `hyprforge-clipmenu` adds `image`/`wayland` to `iced_tiny_skia`) — a
# standalone manifest has to spell that same addition out in full, since
# it has no workspace entry left to extend. A feature the root table
# turns on that the standalone copy has silently lost is the actual
# hazard: a matching version number gives no hint that anything changed.

import glob
import sys
import tomllib

ROOT = "Cargo.toml"


def has_workspace_inheritance(obj):
    if isinstance(obj, dict):
        if obj.get("workspace") is True:
            return True
        return any(has_workspace_inheritance(v) for v in obj.values())
    if isinstance(obj, list):
        return any(has_workspace_inheritance(v) for v in obj)
    return False


def normalize(spec):
    """(version, features:set, default_features:bool), or None if there
    is no version to compare (a path/git-only dependency)."""
    if isinstance(spec, str):
        return (spec, frozenset(), True)
    if isinstance(spec, dict):
        version = spec.get("version")
        if version is None:
            return None
        return (
            version,
            frozenset(spec.get("features", [])),
            spec.get("default-features", True),
        )
    return None


with open(ROOT, "rb") as f:
    root = tomllib.load(f)
root_deps = root.get("workspace", {}).get("dependencies", {})

mismatches = []
checked_crates = 0
checked_deps = 0

for manifest_path in sorted(glob.glob("crates/*/Cargo.toml")):
    with open(manifest_path, "rb") as f:
        doc = tomllib.load(f)

    if has_workspace_inheritance(doc):
        continue  # still inherits from the workspace — not standalone-ready
    if "workspace" in doc and "package" not in doc:
        # A nested workspace (hyprforge-notif): standalone, but its
        # versions are its own on purpose, not copies of this table —
        # it pins zbus to async-io where the suite's carries tokio, and
        # comparing would demand the very feature that panics it. Its own
        # Cargo.lock is what keeps it reproducible.
        continue

    crate_name = doc.get("package", {}).get("name", manifest_path)
    checked_crates += 1
    crate_deps = doc.get("dependencies", {})

    for dep_name, crate_spec in crate_deps.items():
        if dep_name.startswith("hyprforge-"):
            continue  # exempt: git dep here, local path in the root table, by design
        if dep_name not in root_deps:
            continue  # crate-exclusive dependency — no root value to drift from

        root_norm = normalize(root_deps[dep_name])
        crate_norm = normalize(crate_spec)
        if root_norm is None or crate_norm is None:
            continue

        checked_deps += 1
        root_version, root_features, root_default = root_norm
        crate_version, crate_features, crate_default = crate_norm

        if root_version != crate_version:
            mismatches.append(
                f"{crate_name}: {dep_name} version — root={root_version!r} crate={crate_version!r}"
            )
        if root_default != crate_default:
            mismatches.append(
                f"{crate_name}: {dep_name} default-features — root={root_default!r} crate={crate_default!r}"
            )
        missing = root_features - crate_features
        if missing:
            mismatches.append(
                f"{crate_name}: {dep_name} features — root requires {sorted(missing)} "
                f"which is missing from the crate's {sorted(crate_features)}"
            )

if mismatches:
    for m in mismatches:
        print(m)
    sys.exit(1)

print(f"{checked_crates} standalone-ready crate(s), {checked_deps} shared dependency pin(s) checked")

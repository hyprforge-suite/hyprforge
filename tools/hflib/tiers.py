"""The tiers, and the gate each live check waits behind.

The tiers themselves — which test is in which — are filtersets in
`.config/nextest.toml`; this module only knows their names and what has
to be running for each live check to mean anything. Every gate asks the
thing its tests ask, never a neighbour: CLAUDE.md's rule about a check
that silently never runs because it was gated on the wrong service.

`tools/hf lint` holds both halves to each other: every ignored test is
in exactly one tier, and every test in `live` or `visible` sits behind
exactly one gate here — a test no gate names would never run.
"""

import os
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

from .run import have, run

# The tiers that need `--run-ignored only`, in the order a person reads
# them. `unit` is everything not ignored.
IGNORED_TIERS = ["live", "visible", "disposable", "root", "manual"]


def features() -> list[str]:
    """Features every cargo invocation turns on, so one build serves all.

    procman's terminal UI and its eBPF counter are off by default, which
    meant check.sh never compiled — let alone tested — either. Both are
    what the installer builds. netbpf needs clang to compile its BPF
    object; without clang it is left out and `hf check` says so.
    """
    wanted = ["hyprforge-procman/tui"]
    if have("clang"):
        wanted.append("hyprforge-procman/netbpf")
    return wanted


def feature_args() -> list[str]:
    return ["--features", ",".join(features())]


# --- gates ---------------------------------------------------------------


def _systemd_active(unit: str) -> Callable[[], str | None]:
    def check():
        if not have("systemctl"):
            return "systemctl not available to ask"
        if run(["systemctl", "is-active", "--quiet", unit]).ok:
            return None
        return f"{unit} isn't running"

    return check


def _system_bus_name(name: str, what: str) -> Callable[[], str | None]:
    def check():
        if not have("busctl"):
            return "busctl not available to ask"
        r = run(["busctl", "--system", "list", "--acquired", "--activatable", "--no-legend"])
        if any(line.split()[:1] == [name] for line in r.lines()):
            return None
        return f"{what} isn't on the system bus"

    return check


def _user_bus_name(name: str, what: str) -> Callable[[], str | None]:
    def check():
        if not have("busctl"):
            return "busctl not available to ask"
        r = run(["busctl", "--user", "list", "--no-legend"])
        if any(line.split()[:1] == [name] for line in r.lines()):
            return None
        return what

    return check


def _hyprland() -> str | None:
    if not have("hyprctl"):
        return "hyprctl not installed"
    if not run(["hyprctl", "version"]).ok:
        return "Hyprland isn't running"
    return None


def _wayland() -> str | None:
    return None if os.environ.get("WAYLAND_DISPLAY") else "no Wayland session (WAYLAND_DISPLAY unset)"


def _any_program(*names: str) -> Callable[[], str | None]:
    def check():
        return None if any(have(n) for n in names) else f"none of {', '.join(names)} installed"

    return check


def _all_of(*checks: Callable[[], str | None]) -> Callable[[], str | None]:
    def check():
        for c in checks:
            why = c()
            if why:
                return why
        return None

    return check


def _trash() -> str | None:
    data = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share"))
    return None if (data / "Trash/info").is_dir() else "no home trash to read"


def _mime_db() -> str | None:
    data = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share"))
    if Path("/usr/share/mime/globs2").is_file() or (data / "mime/globs2").is_file():
        return None
    return "no shared MIME database installed (shared-mime-info)"


def _portal_xml() -> str | None:
    if not Path("/usr/share/dbus-1/interfaces/org.freedesktop.impl.portal.FileChooser.xml").is_file():
        return "xdg-desktop-portal's FileChooser interface XML is not installed"
    return None if have("dbus-daemon") else "dbus-daemon is not installed"


def _icons() -> str | None:
    return None if have("gsettings") else "gsettings not available to name the icon theme"


def _always() -> str | None:
    return None


@dataclass
class Gate:
    """One live question: which tests ask it, and what must be up first."""

    label: str
    tier: str
    expr: str
    check: Callable[[], str | None]  # None when the gate is open, else why not


GATES: list[Gate] = [
    Gate(
        "Hyprland, read-only",
        "live",
        "(binary_id(hyprforge-appearance::live_appearance) | binary_id(hyprforge-input::live_input)"
        " | binary_id(hyprforge-system::live_system) | binary_id(hyprforge-ecosystem::live_ecosystem))",
        _hyprland,
    ),
    Gate("hyprsunset", "live", "binary_id(hyprforge-ecosystem::live_hyprsunset)", _hyprland),
    Gate(
        "the ecosystem daemons' parsers",
        "live",
        "binary_id(hyprforge-ecosystem::generated_configs_parse)",
        _any_program("hyprpaper", "hypridle"),
    ),
    Gate(
        "the system's archive tools",
        "live",
        "binary_id(hyprforge-archive::live_system_tools)",
        _any_program("tar", "unzip", "7z"),
    ),
    Gate("NetworkManager", "live", "binary_id(hyprforge-network::live_networkmanager)", _systemd_active("NetworkManager")),
    Gate("BlueZ", "live", "binary_id(hyprforge-bluetooth::live_bluez)", _systemd_active("bluetooth")),
    Gate("BlueZ, registering an agent", "visible", "binary_id(hyprforge-bluetooth::live_bluez)", _systemd_active("bluetooth")),
    Gate(
        "systemd-logind",
        "live",
        "(binary_id(hyprforge-power::live_logind) | binary_id(hyprforge-procman) & test(/^sessions::/)"
        " | binary_id(hyprforge-polkit::bin/hyprforge-polkit))",
        _system_bus_name("org.freedesktop.login1", "systemd-logind"),
    ),
    Gate("systemd", "live", "binary_id(hyprforge-procman) & test(/^services::/)", _always),
    Gate("UPower", "live", "binary_id(hyprforge-power::live_upower)", _systemd_active("upower")),
    Gate(
        "power-profiles-daemon",
        "live",
        "binary_id(hyprforge-power::live_power_profiles)",
        _systemd_active("power-profiles-daemon"),
    ),
    Gate(
        "fprintd",
        "live",
        "binary_id(hyprforge-lock::bin/hyprforge-lock)",
        _system_bus_name("net.reactivated.Fprint", "fprintd"),
    ),
    Gate(
        "polkitd",
        "live",
        "binary_id(hyprforge-polkit::live_polkit)",
        _system_bus_name("org.freedesktop.PolicyKit1", "polkitd"),
    ),
    Gate(
        "UDisks2",
        "live",
        "binary_id(hyprforge-volumes::live_udisks)",
        _system_bus_name("org.freedesktop.UDisks2", "UDisks2"),
    ),
    Gate("gio's phones and cameras", "live", "binary_id(hyprforge-volumes::live_gadgets)", _any_program("gio")),
    Gate(
        "the Wayland compositor's protocols",
        "live",
        "(binary_id(hyprforge-clipboard) | binary_id(hyprforge-clipboard::live_clipboard)"
        " | binary_id(hyprforge-displayd::live_smoke) | binary_id(hyprforge-greet::live_text_input))",
        _wayland,
    ),
    Gate("another implementation's trash", "live", "binary_id(hyprforge-fileops::live_trash)", _trash),
    Gate("the installed icon theme", "live", "binary_id(hyprforge-tray::live_icons)", _icons),
    Gate("the shared MIME database", "live", "binary_id(hyprforge-mime::live_mime_database)", _mime_db),
    Gate(
        "the portal's FileChooser interface",
        "live",
        "binary_id(hyprforge-files::live_portal_interface)",
        _portal_xml,
    ),
    Gate("ffmpeg and libmpv", "live", "binary_id(hyprforge-video)", _any_program("ffmpeg")),
    Gate(
        "a tray host",
        "visible",
        "binary_id(hyprforge-tray::live_tray)",
        _user_bus_name("org.kde.StatusNotifierWatcher", "no StatusNotifierWatcher is running (no bar with a tray)"),
    ),
]


def gates_for(tier: str) -> list[Gate]:
    return [g for g in GATES if g.tier == tier]

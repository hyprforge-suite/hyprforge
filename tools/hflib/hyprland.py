"""A disposable nested Hyprland for the tests that change one.

There is no headless-only Hyprland: started without a parent compositor
it goes for the DRM device, which the session already holds, and dies
with `CBackend::create() failed`. So the disposable instance is a nested
one — a window in the session — made as safe as that can be:

  - at most one, named by its config file `hf-disposable.lua`, which
    `crates/hyprforge-lock/testing/nested.sh` (it kills anything whose
    command line says `nested.lua`) cannot mistake for its own;
  - a hard lifetime: it runs under `timeout`, so a run that dies leaves
    a compositor that ends itself within fifteen minutes;
  - found by pid and command line, never `pkill -f` (CLAUDE.md), and
    never the newest directory under /run/user/*/hypr;
  - logs kept in target/hf/logs, not /tmp: the evidence from the
    2026-10-08 freeze was lost with /tmp on the reboot;
  - refuses to start from something that is not a terminal unless told
    `--unattended-ok`, because that freeze happened while an unattended
    agent cycled nested instances. Claude never passes that flag from a
    background agent.

Its hyprpaper and hyprsunset run under the nested instance's signature,
so their sockets are in its own runtime directory and the session's
daemons are never touched.
"""

import json
import os
import signal
import struct
import subprocess
import sys
import time
import zlib
from pathlib import Path

from .run import ROOT, have, run

STATE_DIR = ROOT / "target" / "hf"
STATE = STATE_DIR / "hyprland.json"
LOGS = STATE_DIR / "logs"
CONFIG_NAME = "hf-disposable.lua"
LIFETIME = 15 * 60
HEADLESS = "HF-HEADLESS"

CONFIG = """\
-- The disposable nested Hyprland that `tools/hf` starts for the tests
-- that change a compositor. Written fresh on every start; edit
-- tools/hflib/hyprland.py, not this copy.
--
-- Lua, not .conf: a .conf earns a deprecation banner, and a test
-- compositor that shows warnings teaches everyone to ignore warnings.
-- No binds at all: nothing here should be drivable from the keyboard.
-- The tests reload this file between probes, so it must load with no
-- config errors.
hl.monitor({ output = [[WAYLAND-1]], mode = [[preferred]], position = [[0x0]], scale = 1 })

hl.config({
    misc = {
        disable_hyprland_logo = true,
        disable_splash_rendering = true,
        force_default_wallpaper = 0,
    },
})
"""


def _png(path: Path, size: int = 64) -> None:
    """A plain grey PNG for hyprpaper to show; the fit-mode test needs an
    active wallpaper to re-apply in every mode."""
    raw = b"".join(b"\x00" + b"\x60\x60\x70" * size for _ in range(size))

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw))
        + chunk(b"IEND", b"")
    )


def _alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def _cmdline(pid: int) -> str:
    try:
        return Path(f"/proc/{pid}/cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace")
    except OSError:
        return ""


def _environ(pid: int) -> dict:
    try:
        raw = Path(f"/proc/{pid}/environ").read_bytes()
    except OSError:
        return {}
    out = {}
    for kv in raw.split(b"\0"):
        if b"=" in kv:
            k, v = kv.split(b"=", 1)
            out[k.decode(errors="replace")] = v.decode(errors="replace")
    return out


def _ours(pid: int) -> bool:
    return CONFIG_NAME in _cmdline(pid)


def _instances() -> list[dict]:
    r = run(["hyprctl", "instances", "-j"], timeout=10)
    try:
        return json.loads(r.out) if r.ok else []
    except json.JSONDecodeError:
        return []


def state() -> dict | None:
    """The running disposable instance, or None (and stale state removed)."""
    if not STATE.is_file():
        return None
    try:
        s = json.loads(STATE.read_text())
    except (OSError, json.JSONDecodeError):
        STATE.unlink(missing_ok=True)
        return None
    if not (_alive(s["pid"]) and _ours(s["pid"])):
        STATE.unlink(missing_ok=True)
        return None
    return s


def env_for(s: dict) -> dict:
    """What a test process needs to talk to the disposable instance."""
    return {
        "HYPRFORGE_TEST_HYPRLAND": s["signature"],
        "HYPRFORGE_SESSION_SIGNATURE": s.get("session") or "",
        # Crate code under test (not the testkit) calls bare hyprctl, and
        # some tests speak Wayland directly: both follow these.
        "HYPRLAND_INSTANCE_SIGNATURE": s["signature"],
        "WAYLAND_DISPLAY": s["wl_socket"],
        # The nested clipboard tests name their displays explicitly.
        "HYPRFORGE_TEST_NESTED_DISPLAY": s["wl_socket"],
        "HYPRFORGE_TEST_SESSION_DISPLAY": s.get("host_display") or "",
    }


def up(unattended_ok: bool = False) -> dict:
    existing = state()
    if existing:
        print(f"disposable Hyprland already running: pid {existing['pid']}, {existing['wl_socket']}")
        return existing
    if not sys.stdin.isatty() and not unattended_ok:
        raise SystemExit(
            "refusing to start a nested Hyprland without a terminal attached. On 2026-10-08 the "
            "machine froze while an unattended agent cycled nested instances. Run it from a terminal "
            "with someone present, or pass --unattended-ok if that is really meant."
        )
    host = os.environ.get("WAYLAND_DISPLAY")
    if not host:
        raise SystemExit("no WAYLAND_DISPLAY: a nested Hyprland needs a host session to be a window in")
    if not have("start-hyprland"):
        raise SystemExit("start-hyprland is not installed")
    _sweep()

    work = STATE_DIR / "disposable"
    work.mkdir(parents=True, exist_ok=True)
    LOGS.mkdir(parents=True, exist_ok=True)
    config = work / CONFIG_NAME
    config.write_text(CONFIG)
    log = LOGS / f"hyprland-{time.strftime('%Y%m%d-%H%M%S')}.log"
    before = {i["instance"] for i in _instances()}

    # start-hyprland rather than the binary: the binary alone paints a
    # "started without start-hyprland" banner over the window.
    with open(log, "w") as out:
        subprocess.Popen(
            ["timeout", "--signal=TERM", str(LIFETIME), "start-hyprland", "--", "--config", str(config)],
            stdin=subprocess.DEVNULL,
            stdout=out,
            stderr=subprocess.STDOUT,
            start_new_session=True,
            env={**os.environ, "WAYLAND_DISPLAY": host},
        )

    found = None
    deadline = time.time() + 30
    while time.time() < deadline and not found:
        time.sleep(0.5)
        for inst in _instances():
            if inst["instance"] not in before and _ours(inst["pid"]):
                found = inst
    if not found:
        _sweep()
        raise SystemExit(f"the nested Hyprland did not come up within 30s; its log is {log}")

    sig = found["instance"]
    # A headless output inside the nested instance: a nested window on a
    # workspace the person is not looking at gets no frames from the host,
    # and anything waiting on a frame there waits forever.
    run(["hyprctl", "--instance", sig, "output", "create", "headless", HEADLESS], timeout=10)
    errors = run(["hyprctl", "--instance", sig, "configerrors"], timeout=10).out.strip()

    s = {
        "pid": found["pid"],
        "signature": sig,
        "wl_socket": found["wl_socket"],
        "host_display": host,
        "session": os.environ.get("HYPRLAND_INSTANCE_SIGNATURE", ""),
        "started": time.time(),
        "expires": time.time() + LIFETIME,
        "log": str(log),
        "daemons": {},
    }
    s["daemons"] = _start_daemons(s, work)
    STATE.write_text(json.dumps(s, indent=2))
    print(f"disposable Hyprland up: pid {s['pid']}, {s['wl_socket']}, signature {sig}")
    print(f"  ends itself by {time.strftime('%H:%M', time.localtime(s['expires']))}; log {log}")
    if errors and errors != "no errors":
        print(f"  its config reports errors, which every reload test will trip on:\n{errors}")
    return s


def _start_daemons(s: dict, work: Path) -> dict:
    env = {
        **os.environ,
        "HYPRLAND_INSTANCE_SIGNATURE": s["signature"],
        "WAYLAND_DISPLAY": s["wl_socket"],
    }
    pids = {}
    if have("hyprpaper"):
        image = work / "grey.png"
        _png(image)
        conf = work / "hyprpaper.conf"
        conf.write_text(f"splash = false\nwallpaper {{\n    monitor =\n    path = {image}\n}}\n")
        p = subprocess.Popen(
            ["hyprpaper", "--config", str(conf)],
            stdin=subprocess.DEVNULL,
            stdout=open(LOGS / "hyprpaper.log", "w"),
            stderr=subprocess.STDOUT,
            start_new_session=True,
            env=env,
        )
        pids["hyprpaper"] = p.pid
    if have("hyprsunset"):
        p = subprocess.Popen(
            ["hyprsunset"],
            stdin=subprocess.DEVNULL,
            stdout=open(LOGS / "hyprsunset.log", "w"),
            stderr=subprocess.STDOUT,
            start_new_session=True,
            env=env,
        )
        pids["hyprsunset"] = p.pid
    # Give their sockets a moment to appear; the tests check for them and
    # report a skip, not a pass, if they never did.
    time.sleep(1.5)
    return pids


def _kill(pid: int, check) -> None:
    if not (_alive(pid) and check(pid)):
        return
    os.kill(pid, signal.SIGTERM)
    for _ in range(50):
        if not _alive(pid):
            return
        time.sleep(0.1)
    if check(pid):
        os.kill(pid, signal.SIGKILL)


def _sweep() -> None:
    """Every leftover disposable instance and its daemons, by /proc."""
    for d in Path("/proc").iterdir():
        if not d.name.isdigit():
            continue
        pid = int(d.name)
        if pid == os.getpid():
            continue
        cmd = _cmdline(pid)
        if CONFIG_NAME in cmd and ("Hyprland" in cmd or "start-hyprland" in cmd or cmd.startswith("timeout")):
            _kill(pid, _ours)


def down() -> None:
    s = state()
    if s:
        sig = s["signature"]
        for name, pid in s.get("daemons", {}).items():
            _kill(pid, lambda p, sig=sig: _environ(p).get("HYPRLAND_INSTANCE_SIGNATURE") == sig)
        _kill(s["pid"], _ours)
    _sweep()
    STATE.unlink(missing_ok=True)
    print("disposable Hyprland: none running")


def status() -> None:
    s = state()
    if not s:
        print("disposable Hyprland: none running")
        return
    left = int(s["expires"] - time.time())
    print(f"disposable Hyprland: pid {s['pid']}, {s['wl_socket']}, signature {s['signature']}")
    print(f"  ends itself in {max(left, 0) // 60} min; log {s['log']}")
    for name, pid in s.get("daemons", {}).items():
        print(f"  {name}: pid {pid} {'running' if _alive(pid) else 'gone'}")

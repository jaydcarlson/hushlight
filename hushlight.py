#!/usr/bin/python3
"""Hushlight: lights out, sound off. GNOME + PipeWire, no pip dependencies."""

import argparse
import ctypes
import fcntl
import json
import logging
from logging.handlers import RotatingFileHandler
import os
from pathlib import Path
import signal
import subprocess
import time

LOG = logging.getLogger("hushlight")
DISPLAY = "org.gnome.Mutter.DisplayConfig"
DISPLAY_PATH = "/org/gnome/Mutter/DisplayConfig"


class X11Power:
    """Read the X server directly: Mutter's cached power state can lag wakeup."""

    def __init__(self):
        self.xlib = ctypes.CDLL("libX11.so.6")
        self.ext = ctypes.CDLL("libXext.so.6")
        self.xlib.XOpenDisplay.argtypes = [ctypes.c_char_p]
        self.xlib.XOpenDisplay.restype = ctypes.c_void_p
        self.xlib.XCloseDisplay.argtypes = [ctypes.c_void_p]
        self.ext.DPMSCapable.argtypes = [ctypes.c_void_p]
        self.ext.DPMSCapable.restype = ctypes.c_int
        self.ext.DPMSInfo.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_ushort),
                                     ctypes.POINTER(ctypes.c_int)]
        self.ext.DPMSInfo.restype = ctypes.c_int
        self.connection = self.xlib.XOpenDisplay(None)
        if not self.connection:
            raise RuntimeError("Cannot open X display")
        if not self.ext.DPMSCapable(self.connection):
            self.close()
            raise RuntimeError("X display does not support DPMS")

    def blank(self):
        level, enabled = ctypes.c_ushort(), ctypes.c_int()
        if not self.ext.DPMSInfo(self.connection, ctypes.byref(level), ctypes.byref(enabled)):
            raise RuntimeError("Cannot read X display power state")
        return bool(enabled.value and level.value != 0)

    def close(self):
        if self.connection:
            self.xlib.XCloseDisplay(self.connection)
            self.connection = None


class Screen:
    def __init__(self, bus):
        self.bus = bus
        # Never query XWayland: its DPMS state does not describe real monitors.
        self.x11 = X11Power() if os.environ.get("XDG_SESSION_TYPE") == "x11" else None

    def blank(self):
        return self.x11.blank() if self.x11 else display_blank(self.bus)

    def close(self):
        if self.x11:
            self.x11.close()


def command(*args):
    return subprocess.check_output(args, text=True, timeout=5,
                                   env={**os.environ, "LC_ALL": "C"})


class Audio:
    def snapshot(self):
        objects = decode_dump(command("pw-dump"))
        cookie = next(o["info"]["cookie"] for o in objects
                      if o["type"] == "PipeWire:Interface:Core")
        self.cookie = str(cookie)
        sinks = {}
        for obj in objects:
            info = obj.get("info", {})
            props = info.get("props", {})
            if props.get("media.class") != "Audio/Sink":
                continue
            mute = next((p["mute"] for p in info.get("params", {}).get("Props", [])
                         if isinstance(p.get("mute"), bool)), None)
            if mute is None:
                raise RuntimeError(f"Cannot read mute state for sink {obj['id']}")
            # HDMI nodes disappear on display power-off. Names survive that reconnect;
            # the server cookie prevents restoring records from an old audio server.
            key = f"{cookie}:{props['node.name']}"
            sinks[key] = {"id": obj["id"], "mute": mute,
                          "name": props.get("node.name", str(obj["id"]))}
        return sinks

    def mute(self, key, value):
        # Resolve again just before acting, rather than trusting a stale numeric ID.
        sink = self.snapshot().get(key)
        if sink is None:
            return
        command("wpctl", "set-mute", str(sink["id"]), "1" if value else "0")


def decode_dump(text):
    """Apply every JSON batch: hotplug can emit updates even without --monitor."""
    def merge(old, update):
        for key, value in update.items():
            if isinstance(value, dict) and isinstance(old.get(key), dict):
                merge(old[key], value)
            else:
                old[key] = value

    decoder = json.JSONDecoder()
    objects = {}
    position = 0
    while position < len(text):
        if text[position].isspace():
            position += 1
            continue
        batch, position = decoder.raw_decode(text, position)
        for obj in batch:
            if "info" in obj and obj["info"] is None:
                objects.pop(obj["id"], None)
            else:
                merge(objects.setdefault(obj["id"], {}), obj)
    return list(objects.values())


class Controller:
    def __init__(self, audio, state_path):
        self.audio = audio
        self.path = state_path
        self.saved = json.loads(self.path.read_text()) if self.path.exists() else {}

    def save(self):
        temp = self.path.with_suffix(".tmp")
        temp.write_text(json.dumps(self.saved))
        temp.chmod(0o600)
        temp.replace(self.path)

    def reconcile(self, blank):
        sinks = self.audio.snapshot()
        obsolete = [key for key in self.saved
                    if key.split(":", 1)[0] != self.audio.cookie]
        if obsolete:
            for key in obsolete:
                del self.saved[key]
            self.save()
        if blank:
            for key, sink in sinks.items():
                if key not in self.saved:
                    self.saved[key] = sink["mute"]
                    # Write before muting so a process crash cannot lose the original state.
                    self.save()
                if not sink["mute"]:
                    self.audio.mute(key, True)
                    LOG.info("Muted %s", sink["name"])
        else:
            for key, original in list(self.saved.items()):
                if key not in sinks:
                    continue  # HDMI may reconnect a little after the display wakes.
                if key in sinks and sinks[key]["mute"] != original:
                    self.audio.mute(key, original)
                    LOG.info("Restored %s", sinks[key]["name"])
                del self.saved[key]
                self.save()


def display_blank(bus):
    from gi.repository import Gio, GLib
    result = bus.call_sync(DISPLAY, DISPLAY_PATH, "org.freedesktop.DBus.Properties",
                           "Get", GLib.Variant("(ss)", (DISPLAY, "PowerSaveMode")),
                           GLib.VariantType.new("(v)"), Gio.DBusCallFlags.NONE, 3000, None)
    mode = result.unpack()[0]
    if mode not in (0, 1, 2, 3):
        raise RuntimeError("GNOME cannot report monitor power state")
    return mode != 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Check display/audio without changing anything")
    parser.add_argument("--blank", action="store_true", help="Blank the monitors now, then exit")
    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(levelname)s: %(message)s")
    from gi.repository import Gio, GLib
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    if args.blank:
        # Let the launcher click finish so its release does not immediately wake us.
        time.sleep(0.35)
        bus.call_sync(DISPLAY, DISPLAY_PATH, "org.freedesktop.DBus.Properties", "Set",
                      GLib.Variant("(ssv)", (DISPLAY, "PowerSaveMode", GLib.Variant("i", 3))),
                      None, Gio.DBusCallFlags.NONE, 3000, None)
        return
    screen = Screen(bus)
    audio = Audio()
    if args.check:
        blank = screen.blank()
        sinks = audio.snapshot()
        command("wpctl", "--help")
        print(f"Display: {'blank' if blank else 'awake'}; audio outputs: {len(sinks)}")
        for sink in sinks.values():
            print(f"  {sink['name']}: {'muted' if sink['mute'] else 'unmuted'}")
        screen.close()
        return

    runtime = Path(os.environ["XDG_RUNTIME_DIR"]) / "hushlight"
    runtime.mkdir(mode=0o700, exist_ok=True)
    lock = (runtime / "lock").open("w")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        raise SystemExit("Hushlight is already running")
    log_dir = Path(os.environ.get("XDG_STATE_HOME", Path.home() / ".local/state")) / "hushlight"
    log_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    handler = RotatingFileHandler(log_dir / "hushlight.log", maxBytes=1_000_000, backupCount=1)
    handler.setFormatter(logging.Formatter("%(asctime)s %(levelname)s: %(message)s"))
    LOG.addHandler(handler)
    LOG.info("Watching display through %s", "X11 DPMS" if screen.x11 else "GNOME D-Bus")
    controller = Controller(audio, runtime / "state.json")
    loop = GLib.MainLoop()
    last_error = None
    last_blank = None
    next_audio_check = 0.0

    def tick(*_):
        nonlocal last_error, last_blank, next_audio_check
        try:
            blank = screen.blank()
            # Check display wake promptly, but inspect the audio graph only on
            # transitions or every two seconds while managing muted outputs.
            now = time.monotonic()
            if (blank or controller.saved) and (blank != last_blank or now >= next_audio_check):
                next_audio_check = now + 2.0
                controller.reconcile(blank)
            if blank != last_blank:
                LOG.info("Display %s", "blank — hushing" if blank else "awake")
                last_blank = blank
            last_error = None
        except Exception as exc:
            # Preserve state and retry; an unavailable display is not proof of wakeup.
            if str(exc) != last_error:
                LOG.error("Will retry: %s", exc)
                last_error = str(exc)
        return True

    def stop():
        loop.quit()
        return False

    bus.signal_subscribe(DISPLAY, "org.freedesktop.DBus.Properties", "PropertiesChanged",
                         DISPLAY_PATH, DISPLAY, Gio.DBusSignalFlags.NONE, tick)
    for sig in (signal.SIGINT, signal.SIGTERM):
        GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, sig, stop)
    # Mutter's power cache may lag real X11 wakeup. Poll actual DPMS on X11;
    # use D-Bus on Wayland. Neither check spawns subprocesses.
    GLib.timeout_add(250, tick)
    tick()
    try:
        loop.run()
    finally:
        if controller.saved:
            try:
                controller.reconcile(False)
            except Exception:
                LOG.exception("Restore incomplete; state retained for next start")
        screen.close()


if __name__ == "__main__":
    main()

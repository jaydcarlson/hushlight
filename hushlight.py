#!/usr/bin/python3
"""Hushlight: lights out, sound off. GNOME + PipeWire, no pip dependencies."""

import argparse
import fcntl
import json
import logging
import os
from pathlib import Path
import signal
import subprocess

LOG = logging.getLogger("hushlight")
DISPLAY = "org.gnome.Mutter.DisplayConfig"
DISPLAY_PATH = "/org/gnome/Mutter/DisplayConfig"


def command(*args):
    return subprocess.check_output(args, text=True, timeout=5,
                                   env={**os.environ, "LC_ALL": "C"})


class Audio:
    def snapshot(self):
        objects = json.loads(command("pw-dump"))
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
    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(levelname)s: %(message)s")
    from gi.repository import Gio, GLib
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    audio = Audio()
    if args.check:
        blank = display_blank(bus)
        sinks = audio.snapshot()
        command("wpctl", "--help")
        print(f"Display: {'blank' if blank else 'awake'}; audio outputs: {len(sinks)}")
        for sink in sinks.values():
            print(f"  {sink['name']}: {'muted' if sink['mute'] else 'unmuted'}")
        return

    runtime = Path(os.environ["XDG_RUNTIME_DIR"]) / "hushlight"
    runtime.mkdir(mode=0o700, exist_ok=True)
    lock = (runtime / "lock").open("w")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        raise SystemExit("Hushlight is already running")
    controller = Controller(audio, runtime / "state.json")
    loop = GLib.MainLoop()
    last_error = None
    last_blank = None

    def tick(*_):
        nonlocal last_error, last_blank
        try:
            blank = display_blank(bus)
            # No PipeWire subprocesses while awake unless restoration is pending.
            if blank or controller.saved:
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
    # Retry after service restarts and catch new outputs or manual unmuting while blank.
    GLib.timeout_add_seconds(2, tick)
    tick()
    try:
        loop.run()
    finally:
        if controller.saved:
            try:
                controller.reconcile(False)
            except Exception:
                LOG.exception("Restore incomplete; state retained for next start")


if __name__ == "__main__":
    main()

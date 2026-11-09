# Hushlight

**Lights out, sound off.**

Hushlight mutes your audio outputs when GNOME blanks the screen, then restores
each output's previous mute state when the display wakes. Your computer can keep
running all night without notification sounds waking you up.

- Restores audio on display wake, even if the lock screen is still showing.
- Preserves outputs you had already muted and leaves volume levels alone.
- Mutes all PipeWire audio outputs, including newly connected headphones or HDMI.
- Runs in the background and starts automatically with your desktop session.
- Saves original mute states before changing them, so restarting the app can recover.
- Uses installed system tools; no Python packages to download from PyPI.

## Requirements

GNOME with Mutter's `org.gnome.Mutter.DisplayConfig.PowerSaveMode` interface,
PipeWire with WirePlumber, systemd user services, Python 3.10+, and PyGObject.
Developed on Ubuntu 24.04 / GNOME 46 / X11. The GNOME interface is also available
on Wayland, but that session type has not been tested here.

On Ubuntu, if dependencies are missing:

```sh
sudo apt install python3 python3-gi pipewire-bin wireplumber
```

## Install

From your logged-in GNOME session:

```sh
./install.sh
```

Installation also adds **Hushlight — Good Night** to your applications, with a
moon icon. Click it to immediately turn off the monitors; the running Hushlight
service mutes audio as usual. Move the mouse or press a key to wake the display.
This blanks the monitors without suspending the computer or explicitly locking
the session. To also pin the moon launcher to the GNOME dock:

```sh
./install.sh --pin
```

The same action is available from the command line with
`/usr/bin/python3 hushlight.py --blank`. Audio muting requires the background
service to be running. Uninstalling also removes the launcher and its dock pin.

The installer checks compatibility, copies the app into `~/.local/share/hushlight`,
and enables a user service. No root privileges are needed to install Hushlight.
Run the installer again after pulling updates.

## Use

```sh
systemctl --user status hushlight
journalctl --user -u hushlight -f
tail -f ~/.local/state/hushlight/hushlight.log
systemctl --user stop hushlight      # Pause and restore audio
systemctl --user start hushlight     # Resume
systemctl --user disable --now hushlight  # Stop and disable autostart
./uninstall.sh
```

To inspect compatibility without changing audio:

```sh
/usr/bin/python3 hushlight.py --check
```

To run in the foreground, stop the service first, then run
`/usr/bin/python3 hushlight.py`. Ctrl+C restores audio. A lock prevents two
instances from managing audio simultaneously.

## How it works

Hushlight listens for GNOME display power changes on the session D-Bus. On X11,
it reads the X server's actual DPMS state through libX11/libXext because GNOME's
cached power state can remain "off" after mouse/keyboard input wakes the monitors.
On Wayland, it reads GNOME's power state.
Standby, suspend, or off means mute; on means restore. A lightweight display check
every 250 milliseconds catches wakeups even when GNOME does not emit a power-state
signal. Audio reconciliation runs on display transitions and otherwise at most
every two seconds to catch new outputs, retry temporary failures, and re-mute
outputs that another application unmutes while blank. While awake, it only
queries display state unless audio restoration is pending.

`pw-dump` supplies output identities and mute states; `wpctl` changes mute only.
PipeWire server cookies and stable node names prevent restoring state onto reused
numeric IDs while allowing HDMI outputs to disappear and reconnect when the
monitor powers off. State lives in `$XDG_RUNTIME_DIR/hushlight`, outside the repo.
Stopping the service restores connected outputs. Disconnected outputs' records
are retained until they reconnect and Hushlight can restore them while awake.
A PipeWire server restart discards records from the previous server. Restoration
across reconnect requires the output's node name to remain the same.

This follows **display power state**, not screen locking alone. A monitor's
physical power button may not update the state reported by the desktop. On X11,
DPMS must be enabled; a screensaver overlay without DPMS power-off is not detected.
All audio is muted, including music and calls; microphones are untouched.
Audio sent directly to ALSA or external hardware outside PipeWire is not covered.
There can be a short delay while audio commands run; new outputs are detected on
the next reconciliation. If the display wakes for a notification, audio restores
just as it does on any other display wake.

## Development

```sh
python3 -m unittest discover -s tests -v
```

Tests cover mute restoration, already-muted outputs, hotplug, manual unmuting,
crash recovery, temporary errors, and reused IDs. For a desktop smoke test, run
the service, let GNOME blank the display, then wake it and check the journal and
audio mute state. Test both an initially muted and an initially unmuted output.

Interface references:
[GNOME display power API](https://gitlab.gnome.org/GNOME/mutter/-/blob/46.0/data/dbus-interfaces/org.gnome.Mutter.DisplayConfig.xml),
[pw-dump](https://docs.pipewire.org/page_man_pw-dump_1.html),
[wpctl](https://pipewire.pages.freedesktop.org/wireplumber/man/wpctl.html).

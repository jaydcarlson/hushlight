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

The installer checks compatibility, copies the app into `~/.local/share/hushlight`,
and enables a user service. No root privileges are needed to install Hushlight.
Run the installer again after pulling updates.

## Use

```sh
systemctl --user status hushlight
journalctl --user -u hushlight -f
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

Hushlight listens for GNOME display power changes on the session D-Bus.
Standby, suspend, or off means mute; on means restore. A two-second reconciliation
timer catches new outputs, retries temporary failures, and re-mutes outputs that
another application unmutes while the display is blank. While awake, it only
queries display state unless audio restoration is pending.

`pw-dump` supplies output identities and mute states; `wpctl` changes mute only.
PipeWire server cookies and object serials prevent restoring stale state onto
reused device IDs. State lives in `$XDG_RUNTIME_DIR/hushlight`, outside the repo.
Stopping the service restores connected outputs. Disconnected outputs' records
are discarded; a reconnected device is treated as a new output with its current
mute state. A PipeWire restart likewise creates new output identities.

This follows **GNOME's display power state**, not screen locking alone. A monitor's
physical power button or blanking through tools outside GNOME may not update that
state. All audio is muted, including music and calls; microphones are untouched.
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

# Hushlight

**Lights out, sound off.**

A small Rust daemon that mutes your PipeWire audio outputs when the monitors
blank, then restores their previous mute states when you wake the display.
Your computer can keep running all night without notification sounds waking you.

- Native PipeWire subscriptions for output discovery, hotplug, and mute changes.
- No audio subprocesses, shell commands, or JSON audio snapshots at runtime.
- Preserves already-muted outputs and leaves volume levels and microphones alone.
- Handles HDMI outputs disappearing and reconnecting during monitor power-off.
- Journals mute states before changing them and waits for restoration acknowledgments.
- Starts with your GNOME desktop; includes a moon launcher for bedtime.

## Build and install

Requires a current stable Rust toolchain, GNOME, PipeWire, and systemd user services.
Tested on Ubuntu 24.04 / GNOME 46 / X11 / PipeWire 1.0.5. GNOME Wayland support
uses the same Mutter D-Bus interface as the original version, but is not yet
verified in a Wayland desktop session.

Build dependencies on Ubuntu:

```sh
sudo apt install build-essential pkg-config libclang-dev \
    libpipewire-0.3-dev libspa-0.2-dev libglib2.0-dev
```

Install from your logged-in GNOME session:

```sh
./install.sh --pin
```

Omit `--pin` to install without adding the moon icon to your dock. The installer
builds a release binary, checks compatibility, stops the previous service so it
can restore audio, then installs and starts the replacement. No root privileges
are needed to install Hushlight itself. Run the installer again after pulling updates.

The app, installer helper, and launcher are native Rust; Python is not required.
The executable links against the system PipeWire and GLib/GIO libraries.

## Use

Click **Hushlight — Good Night** in your dock or applications to blank all monitors.
Hushlight mutes the audio automatically. Move the mouse or press a key to wake
the displays and restore sound. This does not suspend the computer or explicitly
lock the session.

```sh
~/.local/share/hushlight/hushlight --check  # Read-only diagnostics
~/.local/share/hushlight/hushlight --blank  # Same as the moon launcher
systemctl --user status hushlight
journalctl --user -u hushlight -f
# Alternative if your account cannot read the journal:
tail -f ~/.local/state/hushlight/hushlight.log
systemctl --user stop hushlight            # Restore audio and pause
systemctl --user start hushlight           # Resume
systemctl --user disable --now hushlight   # Stop and disable autostart
./uninstall.sh
```

To run in a terminal, stop the service and run `target/release/hushlight`.
Ctrl+C restores connected outputs. A shared lock prevents two instances from
changing audio at once, including the previous Python version.

## How it works

**Audio:** A persistent connection to the native PipeWire API subscribes to
`Audio/Sink` nodes and their `Props` parameters. Device and mute events immediately
wake the main loop. Hushlight writes only the node's `mute` property, without
changing routing, channel volumes, or microphone settings. This node-level mute
can differ from the route mute displayed by desktop volume controls; the native
`--check` command reports the state Hushlight actually manages.

**Display:** On X11, Hushlight reads actual DPMS power state directly through the
X protocol. GNOME's cached state can still say "off" after the monitors wake, so
it is deliberately not used for X11 detection. DPMS notifications are enabled
when the X server supports them; older servers use a lightweight 250 ms fallback.
On GNOME Wayland, Hushlight queries Mutter's power-state property every 250 ms.
There is no audio polling subprocess or two-second audio restoration delay.

**Recovery:** Original mute states are saved atomically to
`$XDG_RUNTIME_DIR/hushlight/state.json` before a mute request. Restoration records
are cleared only after PipeWire reports the requested state. A crash/restart
while blank preserves the original states. Server cookies and stable node names
avoid restoring onto reused numeric IDs and allow HDMI reconnects. A new audio
server invalidates the previous server's records. Temporary removal errors do
not terminate the daemon; connection loss causes systemd to restart it.

A normal stop waits up to three seconds for connected outputs to restore.
Disconnected outputs retain their records until they reconnect while Hushlight
is running and the display is awake. Reconnection requires a stable node name.
The Python version's state-file format remains compatible during upgrades.

## Limits

- All audio through managed outputs is muted, including music and calls.
- Audio sent directly to ALSA or external hardware bypassing PipeWire is not covered.
- A monitor's physical power button may not update the desktop's display power state.
- On X11, DPMS must be enabled; a screensaver overlay without power-off is not detected.
- Any display wake, including a notification-induced wake, restores audio.
- The 250 ms fallback plus audio processing sets the wake response time on older X servers.

## Development

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
```

Tests cover preserved mute states, wake-before-mute-acknowledgment races, device
reconnects, server restarts, hotplug, external unmuting, native mute parameter
encoding, state migration, and the distinction between a removed node and a
server disconnect. GitHub Actions builds and checks the native implementation.

Desktop validation should include an audible source, a previously muted output,
HDMI reconnect, termination while blank, and restart after a forced process exit.
Wake the monitors with input or `xset dpms force on` on X11; explicitly resetting
GNOME's cached power state can hide the stale-state bug this implementation avoids.

Interface references:
[PipeWire node API](https://docs.pipewire.org/group__pw__node.html),
[DPMS protocol](https://www.x.org/releases/X11R7.7-RC1/doc/xextproto/dpms.pdf),
[GNOME display power API](https://gitlab.gnome.org/GNOME/mutter/-/blob/46.0/data/dbus-interfaces/org.gnome.Mutter.DisplayConfig.xml).

#!/bin/sh
set -eu
cd -- "$(dirname -- "$0")"
systemctl --user disable --now hushlight.service
"$HOME/.local/share/hushlight/hushlight" --remove-launcher
rm -f "$HOME/.config/systemd/user/hushlight.service" "$HOME/.local/share/hushlight/hushlight" "$HOME/.local/share/hushlight/hushlight.py"
systemctl --user daemon-reload
printf 'Hushlight removed.\n'

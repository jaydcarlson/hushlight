#!/bin/sh
set -eu
systemctl --user disable --now hushlight.service
rm -f "$HOME/.config/systemd/user/hushlight.service" "$HOME/.local/share/hushlight/hushlight.py"
systemctl --user daemon-reload
printf 'Hushlight removed.\n'

#!/bin/sh
set -eu
cd -- "$(dirname -- "$0")"
/usr/bin/python3 hushlight.py --check
mkdir -p "$HOME/.local/share/hushlight" "$HOME/.config/systemd/user"
systemctl --user stop hushlight.service 2>/dev/null || true
install -m 755 hushlight.py "$HOME/.local/share/hushlight/hushlight.py"
install -m 644 hushlight.service "$HOME/.config/systemd/user/hushlight.service"
systemctl --user daemon-reload
systemctl --user enable --now hushlight.service
printf 'Hushlight installed and running. Logs: journalctl --user -u hushlight -f\n'

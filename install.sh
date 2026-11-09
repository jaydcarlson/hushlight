#!/bin/sh
set -eu
cd -- "$(dirname -- "$0")"
case "${1:-}" in ''|--pin) ;; *) printf 'Usage: ./install.sh [--pin]\n' >&2; exit 2;; esac
cargo build --release --locked
./target/release/hushlight --check
mkdir -p "$HOME/.local/share/hushlight" "$HOME/.config/systemd/user"
# Stage the binary before stopping the old version. Renaming also supports updates
# while another process still has the old executable open.
install -m 755 target/release/hushlight "$HOME/.local/share/hushlight/hushlight.new"
if systemctl --user cat hushlight.service >/dev/null 2>&1; then
    systemctl --user stop hushlight.service
fi
mv "$HOME/.local/share/hushlight/hushlight.new" "$HOME/.local/share/hushlight/hushlight"
install -m 644 hushlight.service "$HOME/.config/systemd/user/hushlight.service"
systemctl --user daemon-reload
systemctl --user enable --now hushlight.service
"$HOME/.local/share/hushlight/hushlight" --install-launcher "$@"
printf 'Hushlight installed and running. Logs: journalctl --user -u hushlight -f\n'

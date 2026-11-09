#!/usr/bin/python3
"""Install/remove the moon launcher; optionally pin it to GNOME's dock."""
import argparse
from pathlib import Path
import shutil
from gi.repository import Gio

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--pin", action="store_true")
parser.add_argument("--remove", action="store_true")
args = parser.parse_args()
home = Path.home()
app_id = "hushlight.desktop"
desktop = home / ".local/share/applications" / app_id
icon = home / ".local/share/icons/hicolor/scalable/apps/hushlight.svg"
settings = Gio.Settings.new("org.gnome.shell")
favorites = settings.get_strv("favorite-apps")
if args.remove:
    if app_id in favorites:
        settings.set_strv("favorite-apps", [item for item in favorites if item != app_id])
    desktop.unlink(missing_ok=True)
    icon.unlink(missing_ok=True)
else:
    desktop.parent.mkdir(parents=True, exist_ok=True)
    icon.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(Path(__file__).with_name("hushlight.svg"), icon)
    script = str(home / ".local/share/hushlight/hushlight.py")
    # Desktop Entry Exec quoting is distinct from shell quoting.
    script = script.replace("\\", "\\\\").replace('"', '\\"').replace("`", "\\`").replace("$", "\\$").replace("%", "%%")
    desktop.write_text(
        "[Desktop Entry]\nType=Application\nName=Hushlight — Good Night\n"
        "Comment=Blank the monitors and let Hushlight hush your audio\n"
        f'Exec=/usr/bin/python3 "{script}" --blank\n'
        f"Icon={icon}\nTerminal=false\nStartupNotify=false\n"
        "Categories=Utility;\nKeywords=moon;night;bedtime;screen;mute;\n"
    )
    if args.pin and app_id not in favorites:
        settings.set_strv("favorite-apps", [*favorites, app_id])
Gio.Settings.sync()

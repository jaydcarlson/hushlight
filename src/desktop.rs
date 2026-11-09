use anyhow::{Context, Result};
use gio::prelude::*;
use std::{fs, path::PathBuf};

pub fn install(remove: bool, pin: bool) -> Result<()> {
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is unset")?);
    let app_id = "hushlight.desktop";
    let desktop = home.join(".local/share/applications").join(app_id);
    let icon = home.join(".local/share/icons/hicolor/scalable/apps/hushlight.svg");
    let settings = gio::Settings::new("org.gnome.shell");
    let mut favorites: Vec<String> = settings
        .strv("favorite-apps")
        .iter()
        .map(|s| s.to_string())
        .collect();
    if remove {
        favorites.retain(|s| s != app_id);
        for path in [&desktop, &icon] {
            if path.exists() {
                fs::remove_file(path)?;
            }
        }
    } else {
        fs::create_dir_all(desktop.parent().unwrap())?;
        fs::create_dir_all(icon.parent().unwrap())?;
        fs::write(&icon, include_str!("../hushlight.svg"))?;
        let exe = home.join(".local/share/hushlight/hushlight");
        let exe = exe
            .to_str()
            .context("Home path is not UTF-8")?
            .replace('\\', "\\\\\\\\")
            .replace('"', "\\\\\"")
            .replace('`', "\\\\`")
            .replace('$', "\\\\$")
            .replace('%', "%%");
        fs::write(
            &desktop,
            format!(
                "[Desktop Entry]\nType=Application\nName=Hushlight — Good Night\n\
             Comment=Blank the monitors and let Hushlight hush your audio\n\
             Exec=\"{exe}\" --blank\nIcon={}\nTerminal=false\nStartupNotify=false\n\
             Categories=Utility;\nKeywords=moon;night;bedtime;screen;mute;\n",
                icon.display()
            ),
        )?;
        if pin && !favorites.iter().any(|s| s == app_id) {
            favorites.push(app_id.into());
        }
    }
    settings.set_strv("favorite-apps", favorites)?;
    gio::Settings::sync();
    Ok(())
}

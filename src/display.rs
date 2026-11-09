use anyhow::{Context, Result, bail};
use gio::glib::{self, variant::ToVariant};
use std::os::fd::{AsFd, OwnedFd};
use x11rb::{connection::Connection, protocol::dpms, rust_connection::RustConnection};

const DEST: &str = "org.gnome.Mutter.DisplayConfig";
const PATH: &str = "/org/gnome/Mutter/DisplayConfig";

pub enum Display {
    X11(Box<RustConnection>, bool),
    Gnome(gio::DBusConnection),
}

impl Display {
    pub fn connect() -> Result<Self> {
        if std::env::var("XDG_SESSION_TYPE").as_deref() == Ok("x11") {
            let (conn, _) = x11rb::connect(None).context("Connecting to X11")?;
            if !dpms::capable(&conn)?.reply()?.capable {
                bail!("The X server does not support DPMS");
            }
            let version = dpms::get_version(&conn, 1, 2)?.reply()?;
            let advertised = (version.server_major_version, version.server_minor_version) >= (1, 2);
            // Some older servers echo the requested version but reject SelectInput.
            let events = advertised
                && dpms::select_input(&conn, dpms::EventMask::INFO_NOTIFY)?
                    .check()
                    .is_ok();
            Ok(Self::X11(Box::new(conn), events))
        } else {
            Ok(Self::Gnome(gio::bus_get_sync(
                gio::BusType::Session,
                gio::Cancellable::NONE,
            )?))
        }
    }

    pub fn description(&self) -> &str {
        match self {
            Self::X11(_, true) => "X11 DPMS with power events",
            Self::X11(_, false) => "X11 DPMS (250 ms fallback; server has no power events)",
            Self::Gnome(_) => "GNOME D-Bus (Wayland)",
        }
    }

    pub fn event_fd(&self) -> Result<Option<OwnedFd>> {
        match self {
            Self::X11(conn, true) => Ok(Some(conn.stream().as_fd().try_clone_to_owned()?)),
            _ => Ok(None),
        }
    }

    pub fn blank(&self) -> Result<bool> {
        match self {
            Self::X11(conn, _) => {
                while conn.poll_for_event()?.is_some() {}
                let info = dpms::info(conn.as_ref())?.reply()?;
                Ok(info.state && info.power_level != dpms::DPMSMode::ON)
            }
            Self::Gnome(bus) => {
                let result = bus.call_sync(
                    Some(DEST),
                    PATH,
                    "org.freedesktop.DBus.Properties",
                    "Get",
                    Some(&(DEST, "PowerSaveMode").to_variant()),
                    None,
                    gio::DBusCallFlags::NONE,
                    2000,
                    gio::Cancellable::NONE,
                )?;
                let mode = result
                    .child_value(0)
                    .as_variant()
                    .and_then(|v| v.get::<i32>())
                    .context("Invalid GNOME power state")?;
                match mode {
                    0 => Ok(false),
                    1..=3 => Ok(true),
                    _ => bail!("Unknown GNOME power state"),
                }
            }
        }
    }
}

pub fn blank_now() -> Result<()> {
    let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)?;
    // Allow the dock click to finish before blanking, so its release cannot wake us.
    std::thread::sleep(std::time::Duration::from_millis(350));
    bus.call_sync(
        Some(DEST),
        PATH,
        "org.freedesktop.DBus.Properties",
        "Set",
        Some(&(DEST, "PowerSaveMode", glib::Variant::from(3i32)).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        3000,
        gio::Cancellable::NONE,
    )?;
    Ok(())
}

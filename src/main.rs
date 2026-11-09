mod audio;
mod desktop;
mod display;
mod state;

use anyhow::{Context, Result, bail};
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

static LOG: OnceLock<Mutex<File>> = OnceLock::new();

fn log(message: &str) {
    eprintln!("{message}");
    if let Some(file) = LOG.get()
        && let Ok(mut file) = file.lock()
    {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let _ = writeln!(file, "{now} {message}");
    }
}

fn setup_log() -> Result<()> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap()).join(".local/state"));
    let dir = base.join("hushlight");
    fs::create_dir_all(&dir)?;
    let path = dir.join("hushlight.log");
    if fs::metadata(&path).is_ok_and(|m| m.len() > 1_000_000) {
        fs::rename(&path, dir.join("hushlight.log.1"))?;
    }
    let _ = LOG.set(Mutex::new(
        OpenOptions::new().create(true).append(true).open(path)?,
    ));
    Ok(())
}

fn reconcile(audio: &mut audio::Audio, store: &mut state::Store, blank: bool) -> Result<()> {
    let Some(cookie) = audio.cookie else {
        return Ok(());
    };
    let outputs: Vec<_> = audio
        .sinks
        .iter()
        .filter_map(|(&id, sink)| {
            sink.mute.map(|mute| state::Output {
                id,
                name: sink.name.clone(),
                mute,
                pending: sink.pending.is_some(),
            })
        })
        .collect();
    let (next, commands) = state::plan(&store.saved, cookie, blank, &outputs);
    store.commit(next)?;
    for (id, value) in commands {
        audio.set_mute(id, value)?;
    }
    Ok(())
}

fn run(check: bool) -> Result<()> {
    let display = display::Display::connect()?;
    let mut audio = audio::Audio::connect()?;
    let start = Instant::now();
    while !audio.initialized() {
        audio.dispatch(Duration::from_millis(100))?;
        if start.elapsed() > Duration::from_secs(5) {
            bail!("Timed out reading PipeWire outputs");
        }
    }
    if check {
        println!(
            "Hushlight {} — {}",
            env!("CARGO_PKG_VERSION"),
            display.description()
        );
        println!(
            "Display: {}; audio outputs: {}",
            if display.blank()? { "blank" } else { "awake" },
            audio.sinks.len()
        );
        for sink in audio.sinks.values() {
            println!(
                "  {}: {}",
                sink.name,
                if sink.mute == Some(true) {
                    "muted"
                } else {
                    "unmuted"
                }
            );
        }
        return Ok(());
    }
    let runtime =
        PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is unset")?)
            .join("hushlight");
    fs::create_dir_all(&runtime)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(runtime.join("lock"))?;
    lock.try_lock_exclusive()
        .context("Hushlight is already running")?;
    setup_log()?;
    log(&format!(
        "Hushlight {}: {}",
        env!("CARGO_PKG_VERSION"),
        display.description()
    ));
    let mut store = state::Store::open(runtime.join("state.json"))?;
    let stopping = Arc::new(AtomicBool::new(false));
    for sig in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        signal_hook::flag::register(sig, stopping.clone())?;
    }
    let pwloop = audio.mainloop.clone();
    let _display_io = display.event_fd()?.map(|fd| {
        pwloop
            .loop_()
            .add_io(fd, pipewire::spa::support::system::IoFlags::IN, |_| {})
    });
    let mut last_blank = None;
    let mut last_error = None;
    let result = (|| -> Result<()> {
        while !stopping.load(Ordering::Relaxed) {
            // PipeWire events wake this loop immediately. Only the display fallback polls.
            match display.blank() {
                Ok(blank) => {
                    if last_blank != Some(blank) {
                        log(if blank {
                            "Display blank — hushing"
                        } else {
                            "Display awake — restoring"
                        });
                        last_blank = Some(blank);
                    }
                    reconcile(&mut audio, &mut store, blank)?;
                    last_error = None;
                }
                Err(e) => {
                    let message = e.to_string();
                    if last_error.as_ref() != Some(&message) {
                        log(&format!(
                            "Display unavailable; preserving mute state: {message}"
                        ));
                        last_error = Some(message);
                    }
                }
            }
            audio.dispatch(Duration::from_millis(250))?;
        }
        Ok(())
    })();
    // Restore on a normal stop, with a bounded wait for server acknowledgments.
    // On connection loss retain the journal; systemd will reconnect/restart us.
    if result.is_ok() {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            reconcile(&mut audio, &mut store, false)?;
            let pending_connected = audio.sinks.values().any(|sink| {
                store
                    .saved
                    .contains_key(&format!("{}:{}", audio.cookie.unwrap(), sink.name))
            });
            if !pending_connected || Instant::now() >= deadline {
                break;
            }
            audio.dispatch(Duration::from_millis(50))?;
        }
        log("Stopped; connected outputs restored or retained for retry");
    }
    result
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [] => run(false),
        [arg] if arg == "--check" => run(true),
        [arg] if arg == "--blank" => display::blank_now(),
        [arg] if arg == "--install-launcher" => desktop::install(false, false),
        [arg, pin] if arg == "--install-launcher" && pin == "--pin" => {
            desktop::install(false, true)
        }
        [arg] if arg == "--remove-launcher" => desktop::install(true, false),
        [arg] if arg == "--version" => {
            println!("Hushlight {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        [arg] if arg == "--help" || arg == "-h" => {
            println!(
                "Hushlight — lights out, sound off\n\nRun without options for the background daemon.\n  --check             Inspect display/audio without changes\n  --blank             Blank monitors now\n  --install-launcher [--pin]\n  --remove-launcher\n  --version"
            );
            Ok(())
        }
        _ => Err(anyhow::anyhow!("Unknown arguments; use --help")),
    };
    if let Err(error) = result {
        log(&format!("Error: {error:#}"));
        std::process::exit(1);
    }
}

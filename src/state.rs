use anyhow::Result;
use std::{collections::BTreeMap, fs, path::PathBuf};

pub type Saved = BTreeMap<String, bool>;

pub struct Output {
    pub id: u32,
    pub name: String,
    pub mute: bool,
    pub pending: bool,
}

/// Pure policy: remove restoration records only after the server confirms them.
pub fn plan(
    saved: &Saved,
    cookie: u32,
    blank: bool,
    outputs: &[Output],
) -> (Saved, Vec<(u32, bool)>) {
    let prefix = format!("{cookie}:");
    let mut next: Saved = saved
        .iter()
        .filter(|(k, _)| k.starts_with(&prefix))
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    let mut commands = Vec::new();
    for output in outputs {
        let key = format!("{prefix}{}", output.name);
        if blank {
            next.entry(key).or_insert(output.mute);
            commands.push((output.id, true));
        } else if let Some(&original) = next.get(&key) {
            if output.mute == original && !output.pending {
                next.remove(&key);
            } else {
                commands.push((output.id, original));
            }
        }
    }
    (next, commands)
}

pub struct Store {
    pub saved: Saved,
    path: PathBuf,
}

impl Store {
    pub fn open(path: PathBuf) -> Result<Self> {
        let saved = if path.exists() {
            serde_json::from_slice(&fs::read(&path)?)?
        } else {
            Saved::new()
        };
        Ok(Self { path, saved })
    }

    pub fn commit(&mut self, next: Saved) -> Result<()> {
        if next == self.saved {
            return Ok(());
        }
        let temp = self.path.with_extension("tmp");
        // Persist BEFORE issuing audio commands. Rename is atomic across crashes.
        fs::write(&temp, serde_json::to_vec(&next)?)?;
        fs::File::open(&temp)?.sync_all()?;
        fs::rename(temp, &self.path)?;
        self.saved = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn output(mute: bool, pending: bool) -> Output {
        Output {
            id: 7,
            name: "speaker".into(),
            mute,
            pending,
        }
    }

    #[test]
    fn original_mute_is_preserved_across_repeated_blank_events() {
        for original in [false, true] {
            let (saved, _) = plan(&Saved::new(), 42, true, &[output(original, false)]);
            let (saved, _) = plan(&saved, 42, true, &[output(true, false)]);
            assert_eq!(saved["42:speaker"], original);
            let (restoring, commands) = plan(&saved, 42, false, &[output(true, true)]);
            assert_eq!(commands, vec![(7, original)]);
            assert!(!restoring.is_empty());
            assert!(
                plan(&restoring, 42, false, &[output(original, false)])
                    .0
                    .is_empty()
            );
        }
    }

    #[test]
    fn wake_before_mute_ack_still_sends_unmute() {
        let saved = Saved::from([("42:speaker".into(), false)]);
        let (next, cmds) = plan(&saved, 42, false, &[output(false, true)]);
        assert_eq!(next, saved);
        assert_eq!(cmds, vec![(7, false)]);
    }

    #[test]
    fn disconnected_hdmi_keeps_state_until_reconnect_with_new_id() {
        let saved = Saved::from([("42:speaker".into(), false)]);
        assert_eq!(plan(&saved, 42, false, &[]).0, saved);
        let mut reconnect = output(true, false);
        reconnect.id = 99;
        assert_eq!(plan(&saved, 42, false, &[reconnect]).1, vec![(99, false)]);
    }

    #[test]
    fn new_server_never_restores_an_old_servers_state() {
        let saved = Saved::from([("42:speaker".into(), false)]);
        let (next, commands) = plan(&saved, 43, false, &[output(true, false)]);
        assert!(next.is_empty());
        assert!(commands.is_empty());
    }

    #[test]
    fn hotplug_and_external_unmute_are_hushed() {
        let saved = Saved::from([("42:disconnected".into(), false)]);
        let (saved, cmds) = plan(&saved, 42, true, &[output(false, false)]);
        assert_eq!(cmds, vec![(7, true)]);
        assert_eq!(saved.len(), 2);
        assert_eq!(plan(&saved, 42, true, &[output(false, false)]).1, cmds);
    }

    #[test]
    fn python_state_format_is_compatible() {
        let saved: Saved = serde_json::from_str(r#"{"42:speaker": false}"#).unwrap();
        assert_eq!(
            plan(&saved, 42, false, &[output(true, false)]).1,
            vec![(7, false)]
        );
    }
}

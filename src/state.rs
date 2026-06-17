// Items are wired into main.rs in a subsequent task; suppress dead-code until then.
#![allow(dead_code)]

use color_eyre::eyre::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One persisted port-forward entry. Status is intentionally NOT stored —
/// reloaded tunnels always start Inactive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedTunnel {
    pub machine: String,
    pub local_port: String,
    pub remote_port: String,
}

/// The on-disk shape of `burrow.state.yaml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersistedState {
    #[serde(default)]
    pub tunnels: Vec<PersistedTunnel>,
}

/// Sibling state file next to the config: same directory, `burrow.state.yaml`.
pub fn state_path(config_path: &Path) -> PathBuf {
    match config_path.parent() {
        Some(dir) => dir.join("burrow.state.yaml"),
        None => PathBuf::from("burrow.state.yaml"),
    }
}

/// Tolerant load: a missing or unparseable file yields an empty state rather
/// than an error. The state file is a cache, never critical config.
pub fn load(path: &Path) -> PersistedState {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_norway::from_str(&text).unwrap_or_default(),
        Err(_) => PersistedState::default(),
    }
}

/// Serialize and write the state file.
pub fn save(path: &Path, state: &PersistedState) -> Result<()> {
    let text = serde_norway::to_string(state).wrap_err("serializing state")?;
    std::fs::write(path, text).wrap_err("writing state file")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("az-burrow-state-test-{name}.yaml"))
    }

    #[test]
    fn state_path_is_sibling_of_config() {
        let cfg = Path::new("/home/u/.config/burrow.config.yaml");
        assert_eq!(
            state_path(cfg),
            PathBuf::from("/home/u/.config/burrow.state.yaml")
        );
    }

    #[test]
    fn save_then_load_round_trips() {
        let path = tmp("roundtrip");
        let _ = std::fs::remove_file(&path);
        let state = PersistedState {
            tunnels: vec![PersistedTunnel {
                machine: "vm1".into(),
                local_port: "1234".into(),
                remote_port: "22".into(),
            }],
        };
        save(&path, &state).unwrap();
        let loaded = load(&path);
        assert_eq!(loaded.tunnels, state.tunnels);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_file_loads_empty() {
        let path = tmp("does-not-exist");
        let _ = std::fs::remove_file(&path);
        assert!(load(&path).tunnels.is_empty());
    }

    #[test]
    fn corrupt_file_loads_empty() {
        let path = tmp("corrupt");
        std::fs::write(&path, "this: : is not valid: yaml: [").unwrap();
        assert!(load(&path).tunnels.is_empty());
        let _ = std::fs::remove_file(&path);
    }
}

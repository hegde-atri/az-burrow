use color_eyre::eyre::{eyre, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct MachineConfig {
    pub name: String,
    pub resource_group: String,
    pub target_resource_id: String,
    pub bastion_name: String,
    pub bastion_resource_group: String,
    #[serde(default)]
    pub bastion_subscription: String,
    #[serde(default)]
    pub ssh_config_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub machines: Vec<MachineConfig>,
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        if self.machines.is_empty() {
            return Err(eyre!("no machines defined in config file"));
        }
        Ok(())
    }
}

pub fn parse(text: &str) -> Result<Config> {
    serde_norway::from_str(text).wrap_err("failed to parse config file")
}

/// Read + parse + validate, reproducing Go's LoadOrPrompt error messages.
pub fn load(path: &Path) -> Result<Config> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(eyre!(
                "config file not found at {}\n\nPlease create a burrow.config.yaml file with your Azure VM configurations.\nSee the example in the repository for the required format",
                path.display()
            ));
        }
        Err(e) => return Err(e).wrap_err("failed to read config file"),
    };
    let cfg = parse(&text)?;
    cfg.validate()?;
    Ok(cfg)
}

/// Replicates Go main.go config-path resolution.
/// If `arg` is Some, use it. Otherwise: prefer `burrow.config.yaml` in CWD,
/// then `<home>/.config/burrow.config.yaml`, picking the first that exists;
/// fall back to the first candidate. The result is canonicalized to absolute.
pub fn resolve_config_path(arg: Option<&str>) -> Result<PathBuf> {
    let chosen: PathBuf = if let Some(a) = arg {
        PathBuf::from(a)
    } else {
        let mut candidates = vec![PathBuf::from("burrow.config.yaml")];
        if let Some(h) = home::home_dir() {
            candidates.push(h.join(".config").join("burrow.config.yaml"));
        }
        candidates
            .iter()
            .find(|c| c.exists())
            .cloned()
            .unwrap_or_else(|| candidates[0].clone())
    };
    // Go uses filepath.Abs (does not require the file to exist).
    if chosen.is_absolute() {
        Ok(chosen)
    } else {
        Ok(std::env::current_dir()
            .wrap_err("resolving config path")?
            .join(chosen))
    }
}

/// Expand a leading `~` or `~/` to the home directory. Hardened vs Go's `[2:]`.
pub fn expand_tilde(p: &str) -> String {
    match home::home_dir() {
        Some(h) => expand_tilde_with(p, &h),
        None => p.to_string(),
    }
}

fn expand_tilde_with(p: &str, home: &Path) -> String {
    if p == "~" {
        return home.to_string_lossy().into_owned();
    }
    if let Some(rest) = p.strip_prefix("~/") {
        return home.join(rest).to_string_lossy().into_owned();
    }
    p.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
machines:
  - name: my-vm
    resource_group: MY-RG
    target_resource_id: /subscriptions/x/virtualMachines/my-vm
    bastion_name: my-bastion
    bastion_resource_group: BASTION-RG
    ssh_config_path: ~/.ssh/az_ssh_config/my-vm
  - name: bare-vm
    resource_group: RG2
    target_resource_id: /subscriptions/y/virtualMachines/bare
    bastion_name: b2
    bastion_resource_group: BRG2
"#;

    #[test]
    fn parses_machines_with_optional_fields() {
        let cfg = parse(SAMPLE).unwrap();
        assert_eq!(cfg.machines.len(), 2);
        assert_eq!(cfg.machines[0].name, "my-vm");
        assert_eq!(
            cfg.machines[0].ssh_config_path.as_deref(),
            Some("~/.ssh/az_ssh_config/my-vm")
        );
        // bastion_subscription defaults to empty when omitted
        assert_eq!(cfg.machines[0].bastion_subscription, "");
        // ssh_config_path absent -> None
        assert_eq!(cfg.machines[1].ssh_config_path, None);
    }

    #[test]
    fn empty_machines_is_an_error_via_validate() {
        let cfg = parse("machines: []").unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn expand_tilde_replaces_leading_tilde() {
        let home = std::path::Path::new("/home/test");
        assert_eq!(expand_tilde_with("~/.ssh/x", home), "/home/test/.ssh/x");
        assert_eq!(expand_tilde_with("~", home), "/home/test");
        assert_eq!(expand_tilde_with("/abs/path", home), "/abs/path");
    }
}

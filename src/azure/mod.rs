pub mod cert;
pub mod cleanup;
pub mod parse;
pub mod tunnel;

use tokio::process::Command;

/// Build a [`Command`] that invokes the Azure CLI (`az`).
///
/// On Windows the Azure CLI ships as `az.cmd`, a batch script. Rust's
/// `Command` uses `CreateProcess`, which neither applies `PATHEXT` to a bare
/// `az` nor can launch a batch file directly — so a plain `Command::new("az")`
/// fails with "program not found" even when `az` works in the shell. We route
/// through `cmd /C az`, letting `cmd.exe` resolve and run it exactly as the
/// shell does. On every other platform `az` is a normal executable.
pub fn az_command() -> Command {
    if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.arg("/C").arg("az");
        c
    } else {
        Command::new("az")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn az_command_targets_platform_cli() {
        let cmd = az_command();
        let program = cmd.as_std().get_program().to_string_lossy().into_owned();
        let args: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();

        if cfg!(target_os = "windows") {
            assert_eq!(program, "cmd");
            assert_eq!(args, vec!["/C".to_string(), "az".to_string()]);
        } else {
            assert_eq!(program, "az");
            assert!(args.is_empty());
        }
    }
}

use chrono::{Local, NaiveDateTime, TimeZone};
use color_eyre::eyre::{eyre, Result};
use regex::Regex;

/// Parse `az ssh cert` output: "... is valid until YYYY-MM-DD HH:MM:SS in local time".
/// Returns the expiry as a UTC-based timestamp interpreted in local tz.
pub fn parse_expiry_from_output(output: &str) -> Result<chrono::DateTime<Local>> {
    let re = Regex::new(r"is valid until (\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}) in local time").unwrap();
    let caps = re
        .captures(output)
        .ok_or_else(|| eyre!("could not parse expiry time from output"))?;
    let naive = NaiveDateTime::parse_from_str(&caps[1], "%Y-%m-%d %H:%M:%S")?;
    local_from_naive(naive)
}

/// Parse `ssh-keygen -L -f <cert>` output: "Valid: from ... to YYYY-MM-DDTHH:MM:SS".
pub fn parse_certificate_expiry(output: &str) -> Result<chrono::DateTime<Local>> {
    let re = Regex::new(r"Valid: from .+ to (\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})").unwrap();
    let caps = re
        .captures(output)
        .ok_or_else(|| eyre!("could not parse certificate expiry from ssh-keygen output"))?;
    let naive = NaiveDateTime::parse_from_str(&caps[1], "%Y-%m-%dT%H:%M:%S")?;
    local_from_naive(naive)
}

fn local_from_naive(naive: NaiveDateTime) -> Result<chrono::DateTime<Local>> {
    match Local.from_local_datetime(&naive) {
        chrono::LocalResult::Single(dt) => Ok(dt),
        chrono::LocalResult::Ambiguous(dt, _) => Ok(dt),
        chrono::LocalResult::None => Err(eyre!("invalid local time")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    #[test]
    fn parses_az_output_expiry() {
        let out = "Generated SSH certificate /tmp/x is valid until 2025-10-15 18:06:23 in local time.";
        let t = parse_expiry_from_output(out).unwrap();
        assert_eq!((t.year(), t.month(), t.day()), (2025, 10, 15));
        assert_eq!((t.hour(), t.minute(), t.second()), (18, 6, 23));
    }

    #[test]
    fn az_output_without_marker_errors() {
        assert!(parse_expiry_from_output("nothing here").is_err());
    }

    #[test]
    fn parses_ssh_keygen_validity() {
        let out = "        Valid: from 2025-10-15T17:31:23 to 2025-10-15T18:31:23\n";
        let t = parse_certificate_expiry(out).unwrap();
        assert_eq!((t.hour(), t.minute(), t.second()), (18, 31, 23));
    }
}

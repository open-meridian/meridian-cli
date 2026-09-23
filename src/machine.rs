//! This machine, as the checks see it.
//!
//! Commands are the administrator's own `helm` and `kubectl`, because the
//! chart is what says what runs and an embedded renderer would be a second
//! thing that says it.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::doctor::{Answered, Failure, Machine};

pub struct ThisMachine;

#[async_trait::async_trait]
impl Machine for ThisMachine {
    async fn run(&self, program: &str, arguments: &[&str]) -> Result<String, Failure> {
        let output = match tokio::process::Command::new(program)
            .args(arguments)
            .output()
            .await
        {
            Ok(output) => output,
            // A program that is not here and a program that failed are two
            // different fixes, and only the operating system can tell them
            // apart for us.
            Err(failed) if failed.kind() == std::io::ErrorKind::NotFound => {
                return Err(Failure::Missing(program.to_string()))
            }
            Err(failed) => return Err(Failure::Said(format!("{program}: {failed}"))),
        };

        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        // Both, because kubectl says "no" on stdout and why on stderr.
        Err(Failure::Said(format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }

    async fn fetch(&self, url: &str, headers: &[(&str, &str)]) -> Result<Answered, String> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|failed| failed.to_string())?;
        let mut request = client.get(url);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = request.send().await.map_err(|failed| failed.to_string())?;

        let status = response.status().as_u16();
        let date_s = response
            .headers()
            .get(reqwest::header::DATE)
            .and_then(|value| value.to_str().ok())
            .and_then(http_date);
        let body = response.text().await.unwrap_or_default();
        Ok(Answered {
            status,
            body,
            date_s,
        })
    }

    fn now_s(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_secs())
            .unwrap_or_default()
    }
}

/// `Tue, 23 Sep 2026 00:12:19 GMT`, which is the only date format this needs
/// and the reason there is no date library here.
fn http_date(said: &str) -> Option<u64> {
    let mut parts = said.split_whitespace();
    parts.next()?; // the day name, which says nothing a date does not
    let day: u64 = parts.next()?.parse().ok()?;
    let month = match parts.next()? {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year: u64 = parts.next()?.parse().ok()?;
    let mut clock = parts.next()?.split(':');
    let hours: u64 = clock.next()?.parse().ok()?;
    let minutes: u64 = clock.next()?.parse().ok()?;
    let seconds: u64 = clock.next()?.parse().ok()?;

    // Days since the epoch, by the civil-from-days algorithm, so this needs
    // no table and no leap-year special case beyond the shift.
    let year = if month <= 2 { year - 1 } else { year };
    let era = year / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;

    Some(days * 86_400 + hours * 3_600 + minutes * 60 + seconds)
}

#[cfg(test)]
mod tests {
    use super::http_date;

    #[test]
    fn a_servers_date_is_read_as_seconds_since_the_epoch() {
        // Checked against a known instant rather than against this code's own
        // arithmetic: 2026-09-23T00:12:19Z.
        assert_eq!(
            http_date("Wed, 23 Sep 2026 00:12:19 GMT"),
            Some(1_790_122_339)
        );
        assert_eq!(http_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(
            http_date("Sat, 29 Feb 2020 12:00:00 GMT"),
            Some(1_582_977_600)
        );
    }

    #[test]
    fn anything_else_is_no_date_rather_than_a_wrong_one() {
        assert_eq!(http_date("yesterday"), None);
        assert_eq!(http_date(""), None);
    }
}

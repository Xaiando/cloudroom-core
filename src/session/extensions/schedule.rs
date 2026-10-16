use super::{Error, Result};
use chrono::{Datelike, Timelike};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Schedule {
    Once {
        at_ms: u64,
    },
    Interval {
        every_ms: u64,
    },
    Cron {
        expression: String,
        timezone: String,
    },
}
fn bad() -> Error {
    Error::Conflict(
        "invalid schedule; use a future one-shot, interval, or numeric five-field cron with IANA timezone",
    )
}

fn field(text: &str, low: u32, high: u32) -> Result<Vec<u32>> {
    let mut values = vec![];
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => (range, step.parse::<u32>().map_err(|_| bad())?),
            None => (part, 1),
        };
        if step == 0 || step > high + 1 {
            return Err(bad());
        }
        let (start, end) = if range == "*" {
            (low, high)
        } else if let Some((a, b)) = range.split_once('-') {
            (
                a.parse::<u32>().map_err(|_| bad())?,
                b.parse::<u32>().map_err(|_| bad())?,
            )
        } else {
            let n = range.parse::<u32>().map_err(|_| bad())?;
            (n, if part.contains('/') { high } else { n })
        };
        if start < low || end > high || start > end {
            return Err(bad());
        }
        values.extend((start..=end).step_by(step as usize));
    }
    values.sort_unstable();
    values.dedup();
    if values.is_empty() {
        return Err(bad());
    }
    Ok(values)
}
impl Schedule {
    pub(super) fn next(&self, after: u64) -> Result<Option<u64>> {
        match self {
            Self::Once { at_ms } => {
                if *at_ms <= after {
                    return Ok(None);
                }
                if chrono::DateTime::from_timestamp_millis(*at_ms as i64).is_none()
                    || *at_ms > i64::MAX as u64
                {
                    return Err(bad());
                }
                Ok(Some(*at_ms))
            }
            Self::Interval { every_ms } => {
                let minimum =
                    if std::env::var("CLOUDROOM_UNPROTECTED_TEST_MODE").as_deref() == Ok("1") {
                        1000
                    } else {
                        60_000
                    };
                if *every_ms < minimum || *every_ms > 365 * 86_400_000 {
                    return Err(bad());
                }
                Ok(Some(after.checked_add(*every_ms).ok_or_else(bad)?))
            }
            Self::Cron {
                expression,
                timezone,
            } => {
                if expression.len() > 128 || timezone.len() > 100 {
                    return Err(bad());
                }
                let parts: Vec<_> = expression.split_whitespace().collect();
                if parts.len() != 5 {
                    return Err(bad());
                }
                let fields = [
                    field(parts[0], 0, 59)?,
                    field(parts[1], 0, 23)?,
                    field(parts[2], 1, 31)?,
                    field(parts[3], 1, 12)?,
                    field(parts[4], 0, 7)?,
                ];
                let timezone: chrono_tz::Tz = timezone.parse().map_err(|_| bad())?;
                let mut candidate = after / 60_000 * 60_000 + 60_000;
                for _ in 0..(366 * 5 * 24 * 60) {
                    let utc = chrono::DateTime::from_timestamp_millis(candidate as i64)
                        .ok_or_else(bad)?;
                    let local = utc.with_timezone(&timezone);
                    let dom = fields[2].contains(&local.day());
                    let dow = fields[4]
                        .iter()
                        .any(|d| d % 7 == local.weekday().num_days_from_sunday());
                    let day = if parts[2].starts_with('*') || parts[4].starts_with('*') {
                        dom && dow
                    } else {
                        dom || dow
                    };
                    if day
                        && fields[0].contains(&local.minute())
                        && fields[1].contains(&local.hour())
                        && fields[3].contains(&local.month())
                    {
                        return Ok(Some(candidate));
                    }
                    candidate = candidate.checked_add(60_000).ok_or_else(bad)?;
                }
                Err(bad())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stamp(text: &str) -> u64 {
        chrono::DateTime::parse_from_rfc3339(text)
            .unwrap()
            .timestamp_millis() as u64
    }
    #[test]
    fn timezone_and_dst_use_real_instants() {
        let schedule = Schedule::Cron {
            expression: "30 2 * * *".into(),
            timezone: "Europe/Oslo".into(),
        };
        assert_eq!(
            schedule.next(stamp("2026-03-28T02:00:00Z")).unwrap(),
            Some(stamp("2026-03-30T00:30:00Z"))
        );
        assert_eq!(
            schedule.next(stamp("2026-10-25T00:30:00Z")).unwrap(),
            Some(stamp("2026-10-25T01:30:00Z"))
        );
    }
    #[test]
    fn cron_lists_steps_and_day_or_semantics() {
        let schedule = Schedule::Cron {
            expression: "*/15 9-10 20 * 1".into(),
            timezone: "UTC".into(),
        };
        assert_eq!(
            schedule.next(stamp("2026-10-19T09:01:00Z")).unwrap(),
            Some(stamp("2026-10-19T09:15:00Z"))
        );
        let weekdays = Schedule::Cron {
            expression: "0 9 */1 * 1".into(),
            timezone: "UTC".into(),
        };
        assert_eq!(
            weekdays.next(stamp("2026-10-20T08:00:00Z")).unwrap(),
            Some(stamp("2026-10-26T09:00:00Z"))
        );
        for expr in [
            "0 0 0 * *",
            "0 25 * * *",
            "*/0 * * * *",
            "0 0 * * L",
            "* * * * * *",
        ] {
            assert!(
                Schedule::Cron {
                    expression: expr.into(),
                    timezone: "UTC".into()
                }
                .next(0)
                .is_err()
            );
        }
    }
}

///! Time-related utilities.
use anyhow::anyhow;
use chrono::prelude::*;
use chrono::Duration;

/// The number of seconds in two hours.
const TWO_HOURS_SECS: i64 = 2 * 60 * 60;

fn randomize(fixed_offset: Duration) -> anyhow::Result<DateTime<Utc>> {
    let offset = fixed_offset + Duration::seconds(fastrand::i64(-TWO_HOURS_SECS..TWO_HOURS_SECS));
    Utc::now().checked_add_signed(offset).ok_or_else(|| {
        anyhow!(
            "Failed to add {} to the current datetime, as it will lead to overflow",
            offset
        )
    })
}

/// Random datetime about a day away from now (±2 hours).
pub fn about_a_day_from_now() -> anyhow::Result<DateTime<Utc>> {
    randomize(Duration::days(1))
}

/// Random datetime about a week away from now (±2 hours).
pub fn about_a_week_from_now() -> anyhow::Result<DateTime<Utc>> {
    randomize(Duration::weeks(1))
}

/// Random datetime no further than 24 hours from now.
pub fn sometime_today() -> anyhow::Result<DateTime<Utc>> {
    const DAY: i64 = 24 * 60 * 60;
    let offset = Duration::seconds(fastrand::i64(0..DAY));
    Utc::now().checked_add_signed(offset).ok_or_else(|| {
        anyhow!(
            "Failed to add {} to the current datetime, as it will lead to overflow",
            offset
        )
    })
}

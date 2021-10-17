///! Time-related utilities.
use anyhow::anyhow;
use chrono::prelude::*;

/// The number of seconds in two hours.
const TWO_HOURS_SECS: i64 = 2 * 60 * 60;

/// Random datetime about a day away from now (±2 hours).
pub fn rand_datetime_daily() -> anyhow::Result<DateTime<Utc>> {
    use chrono::Duration;

    let offset =
        Duration::days(1) + Duration::seconds(fastrand::i64(-TWO_HOURS_SECS..TWO_HOURS_SECS));
    Utc::now().checked_add_signed(offset).ok_or_else(|| {
        anyhow!(
            "Failed to add {} to the current datetime, as it will lead to overflow",
            offset
        )
    })
}

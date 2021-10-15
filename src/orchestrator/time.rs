///! Time-related utilities.
use anyhow::anyhow;
use chrono::prelude::*;

/// Random datetime about a day away from now (±10%).
pub fn rand_datetime_daily() -> anyhow::Result<DateTime<Utc>> {
    use chrono::Duration;

    const OFFSET: i64 = 24 * 60 * 60 / 10; // 10% of the day
    let offset = Duration::days(1) + Duration::seconds(fastrand::i64(-OFFSET..OFFSET));
    Utc::now().checked_add_signed(offset).ok_or_else(|| {
        anyhow!(
            "Failed to add {} to the current datetime, as it will lead to overflow",
            offset
        )
    })
}

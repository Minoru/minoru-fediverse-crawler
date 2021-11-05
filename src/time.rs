//! Time-related utilities.
//!
//! We do not want to create undue load on Fediverse instances, but we have to make our HTTP
//! requests *sometime*. To spread the load as evenly as possible, we employ two techniques:
//!
//! 1. *odd periods*, which do not match with common periods like days, weeks, or months. This
//!    helps us avoid a situation where we e.g. always hit an instance just when it's making
//!    a backup;
//! 2. *a bit of randomness*, which helps us avoid a situation where we e.g. hit multiple services
//!    on a single server simultaneously. Even if that happens, randomness will ensure that we
//!    won't do it again (not every time, at least).
//!
//! Note that randomness can work against odd periods, but we'd have to get *extremely* unlucky for
//! that to happen.
//!
//! We have two periods:
//!
//! * *daily*: 29 hours, or a day plus 5 hours;
//! * *weekly*: 167 hours, or a week minus 1 hour.
//!
//! Both of these numbers are prime, so if we have e.g. a "daily" check and a "weekly" check (with
//! no randomization) and those two checks started at the same moment, the next time they'll
//! overlap will be in 29 * 167 hours, or almost 202 days. Sweet!
//!
//! We randomize checks as follows:
//!
//! * *daily*: any number of seconds from -2 hours to 2 hours (both inclusive);
//! * *weekly*: any number of seconds from -11.5 hours to 11.5 hours (both inclusive).
//!
//! These ranges ensure that checks accumulate the same "spread" regardless of their period.
//! There are 167 / 29 ≈ 5.76 "daily" checks per one "weekly" check, so over a "week", a "daily"
//! check will accumulate about 5.76 * 2 ≈ 11.5 hours of "spread" — which is exactly the number of
//! "spread" we give to a "weekly" check.
//!
//! The functions that implement those techniques are [`about_a_day_from_now()`] and
//! [`about_a_week_from_now()`].
//!
//! This module also contains [`sometime_today()`], which is a helper we use when we schedule
//! a check for a newly discovered instance. This is an initial check, so it's not periodic. We
//! still employ randomness though, so when a bunch  of instances are added simultaneously, they
//! won't all get scheduled onto the same time. The amount of randomness is bigger than with the
//! other two functions; it's any number of seconds from 0 to 29 hours (both inclusive).
use anyhow::anyhow;
use chrono::prelude::*;
use chrono::Duration;
use std::ops::RangeBounds;

const DAY_HOURS: i64 = 29;

fn now_plus_offset_plus_random_from_range(
    fixed_offset: Duration,
    range: impl RangeBounds<i64>,
) -> anyhow::Result<DateTime<Utc>> {
    let offset = fixed_offset + Duration::seconds(fastrand::i64(range));
    Utc::now().checked_add_signed(offset).ok_or_else(|| {
        anyhow!(
            "Failed to add {} to the current datetime, as it will lead to overflow",
            offset
        )
    })
}

/// Random datetime about a day from now (now + 29 hours ± 2 hours).
pub fn about_a_day_from_now() -> anyhow::Result<DateTime<Utc>> {
    const TWO_HOURS_SECS: i64 = 2 * 60 * 60;
    now_plus_offset_plus_random_from_range(
        Duration::hours(DAY_HOURS),
        -TWO_HOURS_SECS..=TWO_HOURS_SECS,
    )
}

/// Random datetime about a week away from now (now + 167 hours ± 11.5 hours).
pub fn about_a_week_from_now() -> anyhow::Result<DateTime<Utc>> {
    const ELEVEN_AND_A_HALF_HOURS_SECS: i64 = (11 * 60 + 30) * 60;
    now_plus_offset_plus_random_from_range(
        Duration::hours(167),
        -ELEVEN_AND_A_HALF_HOURS_SECS..=ELEVEN_AND_A_HALF_HOURS_SECS,
    )
}

/// Random datetime no further than 29 hours from now.
pub fn sometime_today() -> anyhow::Result<DateTime<Utc>> {
    const DAY_SECS: i64 = DAY_HOURS * 60 * 60;
    now_plus_offset_plus_random_from_range(Duration::zero(), 0..=DAY_SECS)
}

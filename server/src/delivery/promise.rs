//! The next-day delivery promise, worked out in Africa/Lagos time.

use anyhow::Context;
use sqlx::PgPool;
use time::{Date, Duration, OffsetDateTime, UtcOffset, Weekday};

use crate::error::AppError;

/// Africa/Lagos is UTC+01:00 all year (no DST). Never use the server's local time.
const LAGOS: UtcOffset = match UtcOffset::from_hms(1, 0, 0) {
    Ok(o) => o,
    Err(_) => panic!("valid offset"),
};

/// Bounded: a year of consecutive non-delivery days would be a data error.
const MAX_SKIP: usize = 366;

fn is_delivery_day(d: Date, holidays: &[Date]) -> bool {
    d.weekday() != Weekday::Sunday && !holidays.contains(&d)
}

/// The date an order paid at `paid_at` is promised for. Paid before the cut-off
/// (minutes after local midnight; the exact minute counts as after): the next
/// delivery day; otherwise the delivery day after that. Sundays and `holidays`
/// are skipped; Saturday is a delivery day.
pub fn promised_date(paid_at: OffsetDateTime, cutoff_minutes: i32, holidays: &[Date]) -> Date {
    let local = paid_at.to_offset(LAGOS);
    let minutes = i32::from(local.hour()) * 60 + i32::from(local.minute());
    let mut remaining = if minutes < cutoff_minutes { 1 } else { 2 };
    let mut day = local.date();
    for _ in 0..MAX_SKIP {
        day = day.next_day().expect("date in range");
        if is_delivery_day(day, holidays) {
            remaining -= 1;
            if remaining == 0 {
                break;
            }
        }
    }
    day
}

/// The day a courier is asked to collect a parcel quoted at `now`: today
/// (Africa/Lagos) when it is before the cut-off and a delivery day, otherwise the
/// next delivery day. Sundays and `holidays` are skipped, as in [`promised_date`].
pub fn next_pickup_date(now: OffsetDateTime, cutoff_minutes: i32, holidays: &[Date]) -> Date {
    let local = now.to_offset(LAGOS);
    let minutes = i32::from(local.hour()) * 60 + i32::from(local.minute());
    let mut day = local.date();
    if minutes < cutoff_minutes && is_delivery_day(day, holidays) {
        return day;
    }
    for _ in 0..MAX_SKIP {
        day = day.next_day().expect("date in range");
        if is_delivery_day(day, holidays) {
            break;
        }
    }
    day
}

/// Holidays the owner maintains in `bank_holidays` (from two days back onwards).
pub async fn load_holidays(db: &PgPool) -> Result<Vec<Date>, AppError> {
    let from = OffsetDateTime::now_utc().to_offset(LAGOS).date() - Duration::days(2);
    let days: Vec<Date> =
        sqlx::query_scalar("SELECT day FROM bank_holidays WHERE day >= $1 ORDER BY day LIMIT 1000")
            .bind(from)
            .fetch_all(db)
            .await
            .context("load holidays")?;
    Ok(days)
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::{date, datetime};

    // 2026-11-02 is a Monday.
    #[test]
    fn promised_date_before_cutoff_is_next_day() {
        let paid = datetime!(2026-11-02 17:59 +1);
        assert_eq!(promised_date(paid, 1080, &[]), date!(2026 - 11 - 03));
    }

    #[test]
    fn promised_date_after_cutoff_skips_a_day() {
        let paid = datetime!(2026-11-02 18:00 +1);
        assert_eq!(promised_date(paid, 1080, &[]), date!(2026 - 11 - 04));
    }

    #[test]
    fn promised_date_skips_sunday_and_holidays() {
        // Saturday 2026-10-31 10:00 -> Sunday skipped -> Monday.
        let paid = datetime!(2026-10-31 10:00 +1);
        assert_eq!(promised_date(paid, 1080, &[]), date!(2026 - 11 - 02));
        assert_eq!(
            promised_date(paid, 1080, &[date!(2026 - 11 - 02)]),
            date!(2026 - 11 - 03)
        );
        // Consecutive holidays are all skipped.
        assert_eq!(
            promised_date(paid, 1080, &[date!(2026 - 11 - 02), date!(2026 - 11 - 03)]),
            date!(2026 - 11 - 04)
        );
    }

    #[test]
    fn pickup_is_today_before_cutoff_on_a_delivery_day() {
        let now = datetime!(2026-11-02 17:59 +1);
        assert_eq!(next_pickup_date(now, 1080, &[]), date!(2026 - 11 - 02));
        // Saturday is a delivery day.
        let sat = datetime!(2026-10-31 09:00 +1);
        assert_eq!(next_pickup_date(sat, 1080, &[]), date!(2026 - 10 - 31));
    }

    #[test]
    fn pickup_after_cutoff_is_the_next_delivery_day() {
        // The exact cut-off minute counts as after.
        let now = datetime!(2026-11-02 18:00 +1);
        assert_eq!(next_pickup_date(now, 1080, &[]), date!(2026 - 11 - 03));
        // Saturday evening: Sunday skipped.
        let sat = datetime!(2026-10-31 19:00 +1);
        assert_eq!(next_pickup_date(sat, 1080, &[]), date!(2026 - 11 - 02));
    }

    #[test]
    fn pickup_skips_sundays_and_holidays() {
        // Sunday morning, before the cut-off: Monday.
        let sun = datetime!(2026-11-01 08:00 +1);
        assert_eq!(next_pickup_date(sun, 1080, &[]), date!(2026 - 11 - 02));
        // A holiday today and tomorrow: the day after.
        let mon = datetime!(2026-11-02 08:00 +1);
        assert_eq!(
            next_pickup_date(mon, 1080, &[date!(2026 - 11 - 02), date!(2026 - 11 - 03)]),
            date!(2026 - 11 - 04)
        );
        // Lagos time: 23:30 UTC Monday is 00:30 Tuesday local.
        let late = datetime!(2026-11-02 23:30 UTC);
        assert_eq!(next_pickup_date(late, 1080, &[]), date!(2026 - 11 - 03));
    }

    #[test]
    fn promised_date_uses_lagos_time() {
        let paid = datetime!(2026-11-02 16:59 UTC);
        assert_eq!(promised_date(paid, 1080, &[]), date!(2026 - 11 - 03));
        // 23:30 UTC Monday is 00:30 WAT Tuesday, before the cut-off: promised Wednesday.
        let late = datetime!(2026-11-02 23:30 UTC);
        assert_eq!(promised_date(late, 1080, &[]), date!(2026 - 11 - 04));
    }
}

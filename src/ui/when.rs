use std::time::{SystemTime, UNIX_EPOCH};

const SECONDS_PER_DAY: i64 = 86_400;

pub fn today() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64 / SECONDS_PER_DAY)
}

/// `when` is the `YYYY-MM-DD` a commit summary carries.
pub fn days_ago(when: &str, today: i64) -> Option<i64> {
    let mut fields = when.split('-');
    let year = fields.next()?.parse().ok()?;
    let month = fields.next()?.parse().ok()?;
    let day = fields.next()?.parse().ok()?;

    Some(today - days_from_civil(year, month, day))
}

/// Howard Hinnant's `days_from_civil`, exact for any proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;

    era * 146_097 + day_of_era - 719_468
}

pub fn bucket(days: i64) -> String {
    match days {
        ..=0 => "today".to_owned(),
        1 => "yesterday".to_owned(),
        2..=6 => format!("{days} days ago"),
        7..=13 => "last week".to_owned(),
        14..=30 => plural(days / 7, "week"),
        31..=364 => plural((days / 30).max(1), "month"),
        _ => plural((days / 365).max(1), "year"),
    }
}

fn plural(count: i64, unit: &str) -> String {
    if count == 1 {
        format!("a {unit} ago")
    } else {
        format!("{count} {unit}s ago")
    }
}

/// `06/26/2023 @ 2:48 PM`, on the clock of whoever made the commit: `seconds` is the epoch
/// second and `offset` the seconds east of UTC.
pub fn stamp(seconds: i64, offset: i32) -> String {
    let local = seconds + i64::from(offset);
    let days = local.div_euclid(SECONDS_PER_DAY);
    let rest = local.rem_euclid(SECONDS_PER_DAY);

    let (year, month, day) = civil_from_days(days);
    let (hour, minute) = (rest / 3600, rest % 3600 / 60);
    let (clock, half) = match hour {
        0 => (12, "AM"),
        1..=11 => (hour, "AM"),
        12 => (12, "PM"),
        _ => (hour - 12, "PM"),
    };

    format!("{month:02}/{day:02}/{year} @ {clock}:{minute:02} {half}")
}

/// The inverse of [`days_from_civil`], from the same paper.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };

    (year + i64::from(month <= 2), month, day)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_epoch_is_day_zero() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        assert_eq!(days_from_civil(1970, 12, 31), 364);
        assert_eq!(days_from_civil(2000, 3, 1), 11017);
    }

    #[test]
    fn a_leap_day_counts_once() {
        assert_eq!(
            days_from_civil(2024, 3, 1) - days_from_civil(2024, 2, 28),
            2
        );
        assert_eq!(
            days_from_civil(2023, 3, 1) - days_from_civil(2023, 2, 28),
            1
        );
    }

    #[test]
    fn a_date_is_counted_back_from_today() {
        let today = days_from_civil(2026, 8, 23);

        assert_eq!(days_ago("2026-08-23", today), Some(0));
        assert_eq!(days_ago("2026-08-21", today), Some(2));
        assert_eq!(days_ago("2025-08-23", today), Some(365));
        assert_eq!(days_ago("not a date", today), None);
    }

    #[test]
    fn each_rung_reads_the_way_it_is_spoken() {
        assert_eq!(bucket(0), "today");
        assert_eq!(bucket(1), "yesterday");
        assert_eq!(bucket(3), "3 days ago");
        assert_eq!(bucket(9), "last week");
        assert_eq!(bucket(21), "3 weeks ago");
        assert_eq!(bucket(60), "2 months ago");
        assert_eq!(bucket(400), "a year ago");
        assert_eq!(bucket(1200), "3 years ago");
    }

    #[test]
    fn a_date_ahead_of_today_reads_as_today() {
        assert_eq!(bucket(-5), "today");
    }
}

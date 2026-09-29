// SPDX-License-Identifier: GPL-3.0-or-later
//! Presentation-only English formatting; provider values remain unmodified.

pub fn grouped_count(value: u64) -> String {
    let digits = value.to_string();
    let mut result = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index != 0 && (digits.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(digit);
    }
    result
}

pub fn compact_count(value: u64) -> String {
    let (divisor, suffix) = if value >= 1_000_000_000 {
        (1_000_000_000, "B")
    } else if value >= 1_000_000 {
        (1_000_000, "M")
    } else if value >= 1_000 {
        (1_000, "K")
    } else {
        return value.to_string();
    };
    // Truncate rather than rounding up to imply more subscribers/views.
    let whole = value / divisor;
    let tenth = (value % divisor) / (divisor / 10);
    if whole < 100 && tenth != 0 {
        format!("{whole}.{tenth}{suffix}")
    } else {
        format!("{whole}{suffix}")
    }
}

pub fn date(value: &str) -> String {
    let raw = value.trim();
    if raw.len() != 8
        && !(raw.len() == 10 && raw.as_bytes()[4] == b'-' && raw.as_bytes()[7] == b'-')
    {
        return raw.to_owned();
    }
    let digits: String = raw.chars().filter(|ch| *ch != '-').collect();
    if digits.len() != 8 || !digits.bytes().all(|ch| ch.is_ascii_digit()) {
        return raw.to_owned();
    }
    let year = digits[0..4].parse::<i64>().unwrap_or(0);
    let month = digits[4..6].parse::<u32>().unwrap_or(0);
    let day = digits[6..8].parse::<u32>().unwrap_or(0);
    if !(1..=9999).contains(&year) || !valid_day(year, month, day) {
        return raw.to_owned();
    }
    calendar_date(year, month, day)
}

pub fn timestamp(value: i64) -> String {
    let (year, month, day) = civil_date(value.div_euclid(86_400));
    calendar_date(year, month, day)
}

/// Dates use UTC consistently with stored provider/history timestamps.
pub fn history_day(value: i64, now_seconds: i64) -> String {
    let day = value.div_euclid(86_400);
    let age = now_seconds.div_euclid(86_400).saturating_sub(day);
    match age {
        0 => "Today".to_owned(),
        1 => "Yesterday".to_owned(),
        2..=6 => [
            "Sunday",
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
        ][(day + 4).rem_euclid(7) as usize]
            .to_owned(),
        _ => timestamp(value),
    }
}

fn calendar_date(year: i64, month: u32, day: u32) -> String {
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][(month - 1) as usize];
    format!("{month} {day}, {year}")
}

fn valid_day(year: i64, month: u32, day: u32) -> bool {
    let limit = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => return false,
    };
    (1..=limit).contains(&day)
}

// Proleptic Gregorian conversion from days since 1970-01-01. Integer-only,
// including timestamps before the epoch; no date/runtime dependency required.
fn civil_date(days: i64) -> (i64, u32, u32) {
    let adjusted = days + 719_468;
    let era = adjusted.div_euclid(146_097);
    let day_of_era = adjusted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_and_dates_are_readable_without_inventing_values() {
        assert_eq!(grouped_count(1_234_567), "1,234,567");
        assert_eq!(compact_count(12_345), "12.3K");
        assert_eq!(compact_count(999_999), "999K");
        assert_eq!(compact_count(u64::MAX), "18446744073B");
        assert_eq!(date("2026-09-29"), "Sep 29, 2026");
        assert_eq!(date("20240229"), "Feb 29, 2024");
        assert_eq!(date("2023-02-29"), "2023-02-29");
        assert_eq!(timestamp(0), "Jan 1, 1970");
        assert_eq!(timestamp(-1), "Dec 31, 1969");
        assert_eq!(history_day(0, 86_400), "Yesterday");
        assert_eq!(history_day(0, 172_800), "Thursday");
    }
}

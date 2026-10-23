//! Human-readable times for lists.
use chrono::{DateTime, Datelike, FixedOffset, Local, Offset, TimeZone};

fn at(ts: i64, off: FixedOffset) -> DateTime<FixedOffset> {
    off.timestamp_opt(ts, 0).single().unwrap_or_else(|| off.timestamp_opt(0, 0).unwrap())
}

/// "21:30–22:30" for today, "Wed 21 Oct 21:30–22:30" otherwise. `off` is the viewer's UTC offset.
pub fn range(start: i64, stop: i64, now: i64, off: FixedOffset) -> String {
    let (s, e, n) = (at(start, off), at(stop, off), at(now, off));
    let times = format!("{}–{}", s.format("%H:%M"), e.format("%H:%M"));
    if s.date_naive() == n.date_naive() { times }
    else if (s.date_naive() - n.date_naive()).num_days() == 1 { format!("Tomorrow {times}") }
    else { format!("{} {}", s.format("%a %d %b"), times) }
}

/// "Sat 24" for the day button of a later day.
pub fn day_label(ts: i64, off: FixedOffset) -> String { at(ts, off).format("%a %d").to_string() }

/// "21:30" in the viewer's time zone.
pub fn hm(ts: i64, off: FixedOffset) -> String { at(ts, off).format("%H:%M").to_string() }

pub fn local_offset() -> FixedOffset { Local::now().offset().fix() }
pub fn now_ts() -> i64 { Local::now().timestamp() }

#[allow(dead_code)]
fn _year(ts: i64, off: FixedOffset) -> i32 { at(ts, off).year() }

#[cfg(test)]
mod tests {
    use super::*;
    fn cairo() -> FixedOffset { FixedOffset::east_opt(3 * 3600).unwrap() }
    #[test]
    fn today_tomorrow_and_later() {
        // 2026-10-20 12:00 UTC = 15:00 in UTC+3
        let now = 1_792_497_600;
        assert_eq!(range(now + 3600, now + 7200, now, cairo()), "16:00–17:00");
        assert_eq!(range(now + 86_400, now + 90_000, now, cairo()), "Tomorrow 15:00–16:00");
        assert!(range(now + 3 * 86_400, now + 3 * 86_400 + 3600, now, cairo()).contains("Fri 23 Oct"));
    }
}

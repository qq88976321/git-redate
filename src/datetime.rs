//! Timestamp math for editing commit dates.
//!
//! Git stores each date as unix `seconds` plus a fixed UTC `offset`
//! (seconds east of UTC). We keep that as a small [`Stamp`] which is
//! structurally identical to `gix_date::Time`, converted at the gix
//! boundary so this module depends only on `jiff` and stays testable
//! without a repository.
//!
//! The editor always shows and edits the *wall-clock* time in the
//! commit's own offset. Incrementing a field keeps the offset (and the
//! sub-minute seconds) intact; only [`Component::Offset`] changes the
//! offset, and it does so while holding the displayed wall clock fixed.

use jiff::civil::DateTime;
use jiff::tz::Offset;
use jiff::{Timestamp, ToSpan};

/// A git timestamp: unix seconds plus a fixed UTC offset in seconds
/// east of UTC. Mirrors `gix_date::Time`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub seconds: i64,
    pub offset: i32,
}

/// The editable field of a displayed timestamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Component {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Offset,
}

/// One step of the offset field, in seconds (15 minutes) - fine enough
/// to reach every real-world offset, including the :30 and :45 ones.
const OFFSET_STEP_SECONDS: i32 = 15 * 60;

/// Failed to parse a user-typed `YYYY-MM-DD HH:MM` timestamp.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid date/time (expected YYYY-MM-DD HH:MM): {0}")]
pub struct ParseTimeError(pub String);

impl Stamp {
    pub fn new(seconds: i64, offset: i32) -> Self {
        Stamp { seconds, offset }
    }
}

/// The wall-clock civil datetime in the stamp's own offset.
fn to_civil(stamp: Stamp) -> DateTime {
    // Inputs come from real commits (or from constructors below that
    // already validated), so the conversions cannot fail in practice;
    // fall back to the unix epoch rather than panic.
    let ts = Timestamp::from_second(stamp.seconds).unwrap_or(Timestamp::UNIX_EPOCH);
    let off = Offset::from_seconds(stamp.offset).unwrap_or(Offset::UTC);
    off.to_datetime(ts)
}

/// Reinterpret a civil wall clock under `offset`, yielding unix seconds.
fn seconds_at(dt: DateTime, offset: i32) -> Option<i64> {
    let off = Offset::from_seconds(offset).ok()?;
    Some(off.to_timestamp(dt).ok()?.as_second())
}

/// The `(year, month, day, hour, minute)` parts shown to the user.
pub fn parts(stamp: Stamp) -> (i16, i8, i8, i8, i8) {
    let dt = to_civil(stamp);
    (dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute())
}

/// Format the wall clock as `YYYY-MM-DD HH:MM` in the stamp's offset.
pub fn format(stamp: Stamp) -> String {
    let (y, mo, d, h, mi) = parts(stamp);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}")
}

/// Format the offset as `+HH:MM` / `-HH:MM`.
pub fn format_offset(stamp: Stamp) -> String {
    let secs = stamp.offset;
    let sign = if secs < 0 { '-' } else { '+' };
    let abs = secs.unsigned_abs();
    let (h, m) = (abs / 3600, (abs % 3600) / 60);
    format!("{sign}{h:02}:{m:02}")
}

/// Parse `YYYY-MM-DD HH:MM` and interpret it in `offset`, producing a
/// stamp. Seconds are set to zero (the editor is minute-precision).
pub fn parse_in_offset(text: &str, offset: i32) -> Result<Stamp, ParseTimeError> {
    let err = || ParseTimeError(text.trim().to_string());
    let trimmed = text.trim();
    // Accept either a space or a `T` between date and time.
    let (date, time) = trimmed
        .split_once(' ')
        .or_else(|| trimmed.split_once('T'))
        .ok_or_else(err)?;

    let mut d = date.split('-');
    let year: i16 = next_num(&mut d)?;
    let month: i8 = next_num(&mut d)?;
    let day: i8 = next_num(&mut d)?;
    if d.next().is_some() {
        return Err(err());
    }

    let mut t = time.trim().split(':');
    let hour: i8 = next_num(&mut t)?;
    let minute: i8 = next_num(&mut t)?;
    // Tolerate an optional :SS field but ignore its value.
    if let Some(sec) = t.next() {
        sec.parse::<i8>().map_err(|_| err())?;
    }
    if t.next().is_some() {
        return Err(err());
    }

    let dt = DateTime::new(year, month, day, hour, minute, 0, 0).map_err(|_| err())?;
    let seconds = seconds_at(dt, offset).ok_or_else(err)?;
    Ok(Stamp { seconds, offset })
}

fn next_num<T: std::str::FromStr>(it: &mut std::str::Split<'_, char>) -> Result<T, ParseTimeError> {
    it.next()
        .and_then(|s| s.parse::<T>().ok())
        .ok_or_else(|| ParseTimeError(String::new()))
}

/// Shift a stamp by `delta` seconds, keeping its offset (used by the
/// cascade "shift" edit mode). The wall clock moves with the instant.
pub fn add_delta(stamp: Stamp, delta: i64) -> Stamp {
    Stamp {
        seconds: stamp.seconds.saturating_add(delta),
        offset: stamp.offset,
    }
}

/// Increment/decrement one field by `steps`, with calendar carry
/// (e.g. Jan 31 + 1 month clamps to the month end). The offset and the
/// sub-minute seconds are preserved for the date/time fields; the
/// `Offset` field changes the offset while holding the wall clock.
pub fn bump(stamp: Stamp, component: Component, steps: i64) -> Stamp {
    if component == Component::Offset {
        let delta = OFFSET_STEP_SECONDS as i64 * steps;
        let new_offset = clamp_offset(stamp.offset as i64 + delta);
        // Hold the displayed wall clock; recompute the instant.
        let dt = to_civil(stamp);
        let seconds = seconds_at(dt, new_offset).unwrap_or(stamp.seconds);
        return Stamp {
            seconds,
            offset: new_offset,
        };
    }

    let dt = to_civil(stamp);
    let span = match component {
        Component::Year => steps.years(),
        Component::Month => steps.months(),
        Component::Day => steps.days(),
        Component::Hour => steps.hours(),
        Component::Minute => steps.minutes(),
        Component::Offset => unreachable!("handled above"),
    };
    match dt
        .checked_add(span)
        .ok()
        .and_then(|d| seconds_at(d, stamp.offset))
    {
        Some(seconds) => Stamp {
            seconds,
            offset: stamp.offset,
        },
        // Out of jiff's representable range: leave the stamp untouched.
        None => stamp,
    }
}

/// Valid git/jiff offsets run to +/-18:00; clamp there.
fn clamp_offset(secs: i64) -> i32 {
    secs.clamp(-18 * 3600, 18 * 3600) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2024-03-01 14:32:47 +08:00 -> unix seconds.
    // 14:32:47 +08:00 == 06:32:47 UTC. Computed once and reused.
    fn sample() -> Stamp {
        let dt = DateTime::new(2024, 3, 1, 14, 32, 47, 0).unwrap();
        let seconds = seconds_at(dt, 8 * 3600).unwrap();
        Stamp::new(seconds, 8 * 3600)
    }

    #[test]
    fn format_shows_wall_clock_in_offset() {
        assert_eq!(format(sample()), "2024-03-01 14:32");
        assert_eq!(format_offset(sample()), "+08:00");
    }

    #[test]
    fn format_offset_handles_negative_and_half_hours() {
        assert_eq!(format_offset(Stamp::new(0, -5 * 3600 - 30 * 60)), "-05:30");
        assert_eq!(format_offset(Stamp::new(0, 0)), "+00:00");
        assert_eq!(format_offset(Stamp::new(0, 5 * 3600 + 45 * 60)), "+05:45");
    }

    #[test]
    fn parse_round_trips_format() {
        let s = sample();
        // Reparsing the formatted wall clock zeroes seconds but keeps
        // the same minute in the same offset.
        let reparsed = parse_in_offset(&format(s), s.offset).unwrap();
        assert_eq!(reparsed.offset, s.offset);
        assert_eq!(format(reparsed), "2024-03-01 14:32");
        // Only the sub-minute seconds differ (47 -> 0).
        assert_eq!(s.seconds - reparsed.seconds, 47);
    }

    #[test]
    fn parse_accepts_t_separator_and_optional_seconds() {
        let a = parse_in_offset("2024-03-01 14:32", 0).unwrap();
        let b = parse_in_offset("2024-03-01T14:32", 0).unwrap();
        let c = parse_in_offset("2024-03-01 14:32:59", 0).unwrap();
        assert_eq!(a, b);
        assert_eq!(a, c); // trailing seconds are ignored
    }

    #[test]
    fn parse_rejects_garbage_and_out_of_range() {
        assert!(parse_in_offset("not a date", 0).is_err());
        assert!(parse_in_offset("2024-13-01 00:00", 0).is_err()); // month 13
        assert!(parse_in_offset("2024-02-30 00:00", 0).is_err()); // Feb 30
        assert!(parse_in_offset("2024-01-01 25:00", 0).is_err()); // hour 25
        assert!(parse_in_offset("2024-01-01", 0).is_err()); // no time
    }

    #[test]
    fn parse_interprets_wall_clock_in_given_offset() {
        // Same wall clock, different offsets -> instants differ by the
        // offset difference.
        let utc = parse_in_offset("2024-03-01 12:00", 0).unwrap();
        let plus8 = parse_in_offset("2024-03-01 12:00", 8 * 3600).unwrap();
        assert_eq!(utc.seconds - plus8.seconds, 8 * 3600);
    }

    #[test]
    fn bump_minute_preserves_offset_and_seconds() {
        let s = sample();
        let up = bump(s, Component::Minute, 1);
        assert_eq!(up.offset, s.offset);
        assert_eq!(up.seconds - s.seconds, 60); // +1 minute, seconds kept
        assert_eq!(format(up), "2024-03-01 14:33");
    }

    #[test]
    fn bump_hour_can_roll_into_the_next_day() {
        // 23:32 + 1h -> next day 00:32.
        let base = parse_in_offset("2024-03-01 23:32", 0).unwrap();
        let up = bump(base, Component::Hour, 1);
        assert_eq!(format(up), "2024-03-02 00:32");
    }

    #[test]
    fn bump_month_clamps_to_month_end() {
        // Jan 31 + 1 month -> Feb 29 (2024 is a leap year).
        let jan31 = parse_in_offset("2024-01-31 09:00", 0).unwrap();
        let feb = bump(jan31, Component::Month, 1);
        assert_eq!(format(feb), "2024-02-29 09:00");
    }

    #[test]
    fn bump_negative_steps_go_backward() {
        let s = parse_in_offset("2024-03-01 00:00", 0).unwrap();
        let prev = bump(s, Component::Day, -1);
        assert_eq!(format(prev), "2024-02-29 00:00");
    }

    #[test]
    fn bump_offset_holds_wall_clock_and_changes_instant() {
        // +08:00 wall clock 14:32, step offset up one (15 min) -> the
        // displayed clock stays, offset becomes +08:15, instant shifts.
        let s = sample();
        let up = bump(s, Component::Offset, 1);
        assert_eq!(up.offset, 8 * 3600 + 15 * 60);
        assert_eq!(format(up), "2024-03-01 14:32"); // wall clock held
                                                    // A larger offset (more east) means an earlier instant.
        assert_eq!(s.seconds - up.seconds, 15 * 60);
    }

    #[test]
    fn add_delta_shifts_instant_only() {
        let s = sample();
        let shifted = add_delta(s, 3600);
        assert_eq!(shifted.offset, s.offset);
        assert_eq!(shifted.seconds - s.seconds, 3600);
        assert_eq!(format(shifted), "2024-03-01 15:32");
    }

    #[test]
    fn offset_bump_clamps_at_18_hours() {
        let s = Stamp::new(0, 18 * 3600);
        let up = bump(s, Component::Offset, 10);
        assert_eq!(up.offset, 18 * 3600);
    }
}

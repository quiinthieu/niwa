use tg_core::clock::{Timestamp, TICKS_PER_DAY, TICK_SECS};
use tg_core::season::{resolve_season, Hemisphere, Season, SeasonMode};
use time::{Date, Month, PrimitiveDateTime, Time};

fn utc_ts(year: i32, month: Month, day: u8, hour: u8, minute: u8, second: u8) -> Timestamp {
    let date = Date::from_calendar_date(year, month, day).expect("valid calendar date");
    let time = Time::from_hms(hour, minute, second).expect("valid time");
    Timestamp::from_unix_secs(
        PrimitiveDateTime::new(date, time)
            .assume_utc()
            .unix_timestamp(),
    )
}

#[test]
fn calendar_season_boundaries_hold_for_northern_and_southern_hemispheres() {
    let cases = [
        // Non-leap February -> March 1 boundary
        (
            utc_ts(2026, Month::February, 28, 23, 59, 59),
            Season::Winter,
            Season::Summer,
        ),
        (
            utc_ts(2026, Month::March, 1, 0, 0, 0),
            Season::Spring,
            Season::Autumn,
        ),
        // Leap-year Feb 29 -> March 1 boundary
        (
            utc_ts(2024, Month::February, 28, 23, 59, 59),
            Season::Winter,
            Season::Summer,
        ),
        (
            utc_ts(2024, Month::February, 29, 23, 59, 59),
            Season::Winter,
            Season::Summer,
        ),
        (
            utc_ts(2024, Month::March, 1, 0, 0, 0),
            Season::Spring,
            Season::Autumn,
        ),
        // May 31 -> June 1 boundary
        (
            utc_ts(2026, Month::May, 31, 23, 59, 59),
            Season::Spring,
            Season::Autumn,
        ),
        (
            utc_ts(2026, Month::June, 1, 0, 0, 0),
            Season::Summer,
            Season::Winter,
        ),
        // August 31 -> September 1 boundary
        (
            utc_ts(2026, Month::August, 31, 23, 59, 59),
            Season::Summer,
            Season::Winter,
        ),
        (
            utc_ts(2026, Month::September, 1, 0, 0, 0),
            Season::Autumn,
            Season::Spring,
        ),
        // November 30 -> December 1 boundary
        (
            utc_ts(2026, Month::November, 30, 23, 59, 59),
            Season::Autumn,
            Season::Spring,
        ),
        (
            utc_ts(2026, Month::December, 1, 0, 0, 0),
            Season::Winter,
            Season::Summer,
        ),
        // Year rollover Dec 31 -> Jan 1 stays in mid-season
        (
            utc_ts(2026, Month::December, 31, 23, 59, 59),
            Season::Winter,
            Season::Summer,
        ),
        (
            utc_ts(2027, Month::January, 1, 0, 0, 0),
            Season::Winter,
            Season::Summer,
        ),
    ];

    let dummy_created = Timestamp::from_unix_secs(0);
    for (ts, expected_north, expected_south) in cases {
        let north = resolve_season(
            SeasonMode::Calendar,
            Hemisphere::North,
            ts,
            dummy_created,
            7,
        );
        let south = resolve_season(
            SeasonMode::Calendar,
            Hemisphere::South,
            ts,
            dummy_created,
            7,
        );
        assert_eq!(
            north,
            expected_north,
            "north mismatch at unix={}",
            ts.as_unix_secs()
        );
        assert_eq!(
            south,
            expected_south,
            "south mismatch at unix={}",
            ts.as_unix_secs()
        );
        assert_eq!(north.opposite(), south);
    }
}

#[test]
fn garden_season_mode_cycles_every_configured_season_length_days() {
    let created = Timestamp::from_unix_secs(1_700_000_000);
    let len_days = 7_u32;
    let season_secs = i64::from(len_days) * (TICKS_PER_DAY as i64) * TICK_SECS;

    // Day 0 .. end of Day 6 -> Spring
    assert_eq!(
        resolve_season(
            SeasonMode::Garden,
            Hemisphere::North,
            created,
            created,
            len_days
        ),
        Season::Spring
    );
    assert_eq!(
        resolve_season(
            SeasonMode::Garden,
            Hemisphere::North,
            created.add_secs(season_secs - 1),
            created,
            len_days,
        ),
        Season::Spring
    );

    // Exact boundary into Summer, Autumn, Winter, and wrap back to Spring
    assert_eq!(
        resolve_season(
            SeasonMode::Garden,
            Hemisphere::North,
            created.add_secs(season_secs),
            created,
            len_days,
        ),
        Season::Summer
    );
    assert_eq!(
        resolve_season(
            SeasonMode::Garden,
            Hemisphere::North,
            created.add_secs(2 * season_secs),
            created,
            len_days,
        ),
        Season::Autumn
    );
    assert_eq!(
        resolve_season(
            SeasonMode::Garden,
            Hemisphere::North,
            created.add_secs(3 * season_secs),
            created,
            len_days,
        ),
        Season::Winter
    );
    assert_eq!(
        resolve_season(
            SeasonMode::Garden,
            Hemisphere::North,
            created.add_secs(4 * season_secs),
            created,
            len_days,
        ),
        Season::Spring
    );

    // Backwards clock skew before created_at clamps safely to initial Spring.
    assert_eq!(
        resolve_season(
            SeasonMode::Garden,
            Hemisphere::South,
            created.add_secs(-3_600),
            created,
            len_days,
        ),
        Season::Spring
    );
}

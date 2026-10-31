use tg_core::clock::Clock;
use tg_store::clock::SystemClock;

#[test]
fn system_clock_returns_plausible_monotonic_unix_timestamp() {
    let clock = SystemClock;
    let first = clock.now();
    let second = clock.now();

    assert!(
        first.as_unix_secs() >= 1_700_000_000,
        "SystemClock returned implausibly old timestamp: {}",
        first.as_unix_secs()
    );
    assert!(
        second >= first,
        "SystemClock moved backwards unexpectedly: {} < {}",
        second.as_unix_secs(),
        first.as_unix_secs()
    );
}

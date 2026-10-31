#[path = "../../../tests/support/mod.rs"]
mod support;

use std::time::Duration;
use support::TestEnv;
use tg_core::clock::Timestamp;
use tg_signals::activity::{
    activity_event_from_idle, freedesktop_screensaver_idle_duration, is_idle_duration_active,
    parse_gdbus_uint_tuple, parse_ioreg_hid_idle_ns, parse_xprintidle_ms,
    sample_and_record_activity, ActivitySampleOutcome, FakeIdleSource, ACTIVITY_ACTIVE_WINDOW,
};
use tg_signals::consent::any_signal_enabled;
use tg_signals::{EventSpool, SignalKind, WorkEvent};
use tg_store::config::AppConfig;

#[test]
fn any_signal_enabled_requires_explicit_enable_and_consent_timestamp() {
    let mut config = AppConfig::default();
    assert!(!any_signal_enabled(&config));

    // Corrupt/manual flag without `consented_at` still counts as disabled.
    config.signals.activity.enabled = true;
    config.signals.activity.consented_at = None;
    assert!(!any_signal_enabled(&config));

    config.enable_signal(SignalKind::Git, Timestamp::from_unix_secs(1_700_000_000));
    assert!(any_signal_enabled(&config));

    config.disable_signal(SignalKind::Git, false);
    assert!(!any_signal_enabled(&config));
}

#[test]
fn idle_threshold_classifies_under_60s_as_active_and_60s_or_more_as_idle() {
    assert_eq!(ACTIVITY_ACTIVE_WINDOW, Duration::from_secs(60));
    assert!(is_idle_duration_active(Duration::ZERO));
    assert!(is_idle_duration_active(Duration::from_millis(59_999)));
    assert!(!is_idle_duration_active(Duration::from_secs(60)));
    assert!(!is_idle_duration_active(Duration::from_secs(300)));

    let ts = Timestamp::from_unix_secs(1_700_000_060);
    assert_eq!(
        activity_event_from_idle(ts, Duration::from_secs(12)),
        WorkEvent::Activity { ts, active: true }
    );
    assert_eq!(
        activity_event_from_idle(ts, Duration::from_secs(60)),
        WorkEvent::Activity { ts, active: false }
    );
}

#[test]
fn platform_idle_output_parsers_handle_macos_ioreg_and_linux_dbus_xprintidle() {
    let sample_ioreg = r#"
+-o IOHIDSystem  <class IOHIDSystem, id 0x1000005a8, registered, matched, active, busy 0 (0 ms), retain 29>
    {
      "HIDIdleTime" = 14250000000
      "IOGeneralInterest" = "IOCommand is not serializable"
    }
"#;
    assert_eq!(
        parse_ioreg_hid_idle_ns(sample_ioreg),
        Some(Duration::from_millis(14_250))
    );
    assert_eq!(parse_ioreg_hid_idle_ns("no idle key here"), None);
    assert_eq!(
        parse_ioreg_hid_idle_ns("\"HIDIdleTime\" = not_a_number"),
        None
    );

    // GNOME Mutter IdleMonitor (`(uint64 <ms>,)`) and freedesktop ScreenSaver (`(uint32 <val>,)`).
    assert_eq!(parse_gdbus_uint_tuple("(uint64 48250,)\n"), Some(48_250));
    assert_eq!(parse_gdbus_uint_tuple("(uint32 19,)\n"), Some(19));
    assert_eq!(parse_gdbus_uint_tuple("(0,)"), Some(0));
    assert_eq!(parse_gdbus_uint_tuple("Error: GDBus.Error"), None);

    // freedesktop ScreenSaver unit handling (seconds per spec vs milliseconds on KDE Plasma).
    assert_eq!(
        freedesktop_screensaver_idle_duration(15, Some("GNOME")),
        Duration::from_secs(15)
    );
    assert_eq!(
        freedesktop_screensaver_idle_duration(15, None),
        Duration::from_secs(15)
    );
    assert_eq!(
        freedesktop_screensaver_idle_duration(15_000, Some("KDE")),
        Duration::from_millis(15_000)
    );
    assert_eq!(
        freedesktop_screensaver_idle_duration(15_000, Some("ubuntu:kde-plasma")),
        Duration::from_millis(15_000)
    );

    // X11 `xprintidle` stdout (`<ms>\n`).
    assert_eq!(
        parse_xprintidle_ms("7320\n"),
        Some(Duration::from_millis(7_320))
    );
    assert_eq!(parse_xprintidle_ms(""), None);
    assert_eq!(parse_xprintidle_ms("couldn't open display"), None);
}

#[test]
fn sample_and_record_activity_enforces_consent_tick_completion_and_spool_dedup() {
    let env = TestEnv::new();
    let spool = EventSpool::new(env.paths().clone());
    let t0 = Timestamp::from_unix_secs(1_700_000_000);
    let t1 = t0.add_secs(60);

    // 1. Disabled signal: `FakeIdleSource` must NOT be polled and nothing is written.
    let mut config = AppConfig::default();
    let fake = FakeIdleSource::constant(Some(Duration::from_secs(5)));
    let outcome = sample_and_record_activity(&config, &spool, &[], Some(t0), t1, &fake).unwrap();
    assert_eq!(outcome, ActivitySampleOutcome::Disabled);
    assert_eq!(fake.calls(), 0);
    assert!(!env.paths().events_file().exists());

    // 2. Enabled signal, but `< 60 s` elapsed (`NoCompletedTick`): `FakeIdleSource` not polled.
    config.enable_signal(SignalKind::Activity, t0);
    let outcome =
        sample_and_record_activity(&config, &spool, &[], Some(t0), t0.add_secs(45), &fake).unwrap();
    assert_eq!(outcome, ActivitySampleOutcome::NoCompletedTick);
    assert_eq!(fake.calls(), 0);

    // 3. Enabled signal at `now = t0 + 62` (with +2 s timer coalescing jitter):
    //    stamps `ts = t1` (`t0 + 60`, the completed tick boundary) with `active = true`.
    let outcome =
        sample_and_record_activity(&config, &spool, &[], Some(t0), t0.add_secs(62), &fake).unwrap();
    assert_eq!(
        outcome,
        ActivitySampleOutcome::Recorded {
            ts: t1,
            active: true
        }
    );
    assert_eq!(fake.calls(), 1);
    let events = spool.read().unwrap().events;
    assert_eq!(
        events,
        vec![WorkEvent::Activity {
            ts: t1,
            active: true
        }]
    );

    // 4. Calling again in the same tick window (or when Swift `Niwa.app` already queued an
    //    `Activity` event for `(t0, t1]`): skips polling `IdleSource`!
    let outcome_dup =
        sample_and_record_activity(&config, &spool, &events, Some(t0), t0.add_secs(65), &fake)
            .unwrap();
    assert_eq!(outcome_dup, ActivitySampleOutcome::AlreadyRecorded);
    assert_eq!(fake.calls(), 1);

    // 5. Unavailable idle source (`None`): records no fabricated event.
    let unavailable = FakeIdleSource::unavailable();
    let t2 = t1.add_secs(60);
    let outcome_none =
        sample_and_record_activity(&config, &spool, &[], Some(t1), t2, &unavailable).unwrap();
    assert_eq!(outcome_none, ActivitySampleOutcome::SourceUnavailable);
    assert_eq!(unavailable.calls(), 1);
}

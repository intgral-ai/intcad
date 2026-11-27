use std::cell::Cell;
use std::time::Duration;

use super::SuspendClock;

#[test]
fn forward_readings_preserve_precision_and_jumps() {
    let clock = SuspendClock::new(|| None);
    let first = Duration::new(10, 123);
    let after_suspend = Duration::new(130, 456);
    for reading in [first, first, after_suspend] {
        assert_eq!(clock.read_with(|| Some(reading)), Some(reading));
    }
}

#[test]
fn a_zero_first_reading_cannot_keep_a_deadline_alive() {
    let clock = SuspendClock::new(|| Some(Duration::ZERO));
    // The account adapters map None to Duration::MAX and use a half-open deadline.
    let reading = clock.read();
    let now = reading.unwrap_or(Duration::MAX);
    let expires_at = now.saturating_add(Duration::from_secs(30));
    assert!(now >= expires_at, "a zero reading must not start a live deadline");
    assert_eq!(reading, None);
}

#[test]
fn a_zero_reading_stays_failed_even_if_the_source_recovers() {
    let positive = Duration::from_secs(10);
    for has_previous in [false, true] {
        let clock = SuspendClock::new(|| Some(Duration::ZERO));
        if has_previous {
            assert_eq!(clock.read_with(|| Some(positive)), Some(positive));
        }
        assert_eq!(clock.read(), None);
        assert_eq!(clock.read_with(|| Some(Duration::from_secs(11))), None);
    }
}

#[test]
fn accepted_readings_preserve_the_exact_source_intervals() {
    thread_local! {
        static SOURCE: Cell<Duration> = const { Cell::new(Duration::ZERO) };
    }
    let clock = SuspendClock::new(|| Some(SOURCE.get()));
    let start = Duration::new(10, 123);
    let samples = [
        (Duration::new(10, 128), Duration::from_nanos(5)),
        (Duration::new(130, 456), Duration::new(120, 328)),
        (Duration::new(130, 456), Duration::ZERO),
    ];
    SOURCE.set(start);
    let mut previous = clock.read();
    assert_eq!(previous, Some(start));
    for (reading, expected_delta) in samples {
        SOURCE.set(reading);
        let reported = clock.read();
        let delta = reported.zip(previous).and_then(|(now, before)| now.checked_sub(before));
        assert_eq!(delta, Some(expected_delta));
        previous = reported;
    }
}

#[test]
fn unavailable_source_permanently_invalidates_the_clock() {
    let clock = SuspendClock::new(|| None);
    assert_eq!(clock.read(), None);
    assert_eq!(clock.read_with(|| Some(Duration::from_secs(10))), None);
}

#[test]
fn backwards_step_permanently_invalidates_the_clock() {
    let clock = SuspendClock::new(|| None);
    assert_eq!(clock.read_with(|| Some(Duration::from_secs(10))), Some(Duration::from_secs(10)));
    assert_eq!(clock.read_with(|| Some(Duration::from_secs(9))), None);
    assert_eq!(clock.read_with(|| Some(Duration::from_secs(11))), None);
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[test]
fn supported_native_platform_exposes_a_readable_clock() {
    let clock = SuspendClock::default();
    let first = clock.read().expect("native continuous clock must be available on the supported runner");
    let limit = std::time::Instant::now() + Duration::from_secs(1);
    loop {
        let next = clock.read().expect("the native continuous clock must stay available");
        if next > first {
            break;
        }
        assert!(std::time::Instant::now() < limit, "the native continuous clock must advance");
        std::thread::sleep(Duration::from_millis(1));
    }
}

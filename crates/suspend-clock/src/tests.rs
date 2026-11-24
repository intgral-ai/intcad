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
    let first = clock.read();
    assert!(first.is_some(), "native continuous clock must be available on the supported runner");
    assert!(clock.read() >= first);
}

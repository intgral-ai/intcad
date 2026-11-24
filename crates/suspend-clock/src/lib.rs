//! Native elapsed time that includes suspend and is independent of wall-clock adjustments.

#![deny(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::sync::Mutex;
use std::time::Duration;

#[cfg(test)]
mod tests;

/// An elapsed clock with a fixed platform epoch, not relative to construction.
/// A missing or backwards reading permanently invalidates this instance.
pub struct SuspendClock {
    source: fn() -> Option<Duration>,
    last: Mutex<Option<Duration>>,
}

impl Default for SuspendClock {
    fn default() -> Self {
        Self::new(native_read)
    }
}

impl SuspendClock {
    /// Use a caller-supplied suspend-inclusive source, also used to exercise failures in tests.
    pub fn new(source: fn() -> Option<Duration>) -> Self {
        Self { source, last: Mutex::new(Some(Duration::ZERO)) }
    }

    /// Return the reading unchanged, or `None` if the clock cannot be trusted.
    pub fn read(&self) -> Option<Duration> {
        self.read_with(self.source)
    }

    fn read_with(&self, source: impl FnOnce() -> Option<Duration>) -> Option<Duration> {
        // Serialize the source call as well as the comparison: a delayed caller cannot
        // mistake an older concurrent sample for a backward step.
        let mut last = self.last.lock().ok()?;
        let previous = (*last)?;
        *last = source().filter(|sample| *sample >= previous);
        *last
    }
}

#[cfg(target_os = "linux")]
fn native_read() -> Option<Duration> {
    let time = rustix::time::clock_gettime_dynamic(rustix::time::DynamicClockId::Boottime).ok()?;
    let seconds = u64::try_from(time.tv_sec).ok()?;
    let nanos = u32::try_from(time.tv_nsec).ok().filter(|nanos| *nanos < 1_000_000_000)?;
    Some(Duration::new(seconds, nanos))
}

#[cfg(target_os = "macos")]
fn native_read() -> Option<Duration> {
    let mut time = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: a valid clock ID and a writable timespec; the return code reports failure.
    // Darwin clock_gettime(3)'s CLOCK_MONOTONIC_RAW reads mach_continuous_time, already
    // converted to seconds/nanoseconds. It is monotonic, includes sleep and ignores wall
    // adjustments. libc has no safe wrapper; std::time::Instant uses the suspending clock.
    #[allow(unsafe_code)]
    let status = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, &mut time) };
    if status != 0 {
        return None;
    }
    let seconds = u64::try_from(time.tv_sec).ok()?;
    let nanos = u32::try_from(time.tv_nsec).ok().filter(|nanos| *nanos < 1_000_000_000)?;
    Some(Duration::new(seconds, nanos))
}

#[cfg(target_os = "windows")]
fn native_read() -> Option<Duration> {
    use std::sync::OnceLock;

    type QueryInterruptTimePrecise = unsafe extern "system" fn(*mut u64);
    static QUERY: OnceLock<Option<QueryInterruptTimePrecise>> = OnceLock::new();

    // Resolve at runtime so an absent export leaves billing closed, not the executable unloadable.
    #[allow(unsafe_code)]
    fn resolve() -> Option<QueryInterruptTimePrecise> {
        use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};

        // SAFETY: both names are NUL-terminated ASCII. Kernel32 stays loaded for the process
        // lifetime; the exact name identifies the realtimeapiset.h export with the ABI above.
        // windows-sys has no safe API for this optional entry point.
        let kernel = unsafe { GetModuleHandleA(c"kernel32.dll".as_ptr().cast()) };
        if kernel.is_null() {
            return None;
        }
        let symbol = unsafe { GetProcAddress(kernel, c"QueryInterruptTimePrecise".as_ptr().cast()) }?;
        Some(unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, QueryInterruptTimePrecise>(symbol) })
    }

    let query = (*QUERY.get_or_init(resolve))?;
    let mut ticks = 0;
    // SAFETY: the resolved function writes a ULONGLONG to this valid out pointer and
    // has no error return. Microsoft documents biased interrupt time as monotonic since
    // boot, including sleep/hibernation and unaffected by wall-clock changes. std has
    // no safe wrapper for this source.
    #[allow(unsafe_code)]
    let () = unsafe { query(&mut ticks) };
    const TICKS_PER_SECOND: u64 = 10_000_000;
    Some(Duration::new(ticks / TICKS_PER_SECOND, ((ticks % TICKS_PER_SECOND) * 100) as u32))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn native_read() -> Option<Duration> {
    None
}

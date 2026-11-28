//! Windows and macOS: the memory available now, for the stress estimate's memory gates (decision 22
//! of `docs/design/fea-engineering-ready.md`). The engine reads Linux's `/proc/meminfo` itself;
//! here [`available`] is handed to it at start (`solidcraft_engine::fea::set_memory_reader`). The
//! CLI has no reader, and its gates assume 8 GB a run on these systems.
//!
//! Pure functions turn each system's numbers into bytes and are tested on every system. Asking the
//! system has no safe API, so only the two functions that do allow `unsafe_code` (the app crate
//! denies it; see `open_documents.rs` for the same).

/// Windows: the bytes available from `GlobalMemoryStatusEx`'s `ullAvailPhys` (physical memory free
/// or on the standby list, which the system hands back at once: Task Manager's "Available") and
/// `ullAvailPageFile` (what this process may still commit: past it an allocation fails, with memory
/// free or not). The smaller of the two; 0 is no reading.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn windows_available(avail_phys: u64, avail_commit: u64) -> Option<f64> {
    let phys = (avail_phys > 0).then_some(avail_phys as f64)?;
    Some(if avail_commit > 0 { phys.min(avail_commit as f64) } else { phys })
}

/// macOS: the bytes available from the physical memory (`hw.memsize`, bytes) and the share of it
/// the kernel counts as available (`kern.memorystatus_level`, %: the free memory and the file pages
/// it can drop, the "memory free percentage" `memory_pressure` prints). `None` for no memory or a
/// share outside 1 to 100.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn mac_available(memsize: u64, percent: u64) -> Option<f64> {
    (memsize > 0 && (1..=100).contains(&percent)).then(|| memsize as f64 * percent as f64 / 100.0)
}

/// A number `sysctlbyname` wrote: 4 or 8 bytes in this computer's byte order (an `int` or an
/// `int64`). `None` for any other length.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn sysctl_number(bytes: &[u8]) -> Option<u64> {
    match bytes.len() {
        4 => Some(u32::from_ne_bytes(bytes.try_into().ok()?).into()),
        8 => Some(u64::from_ne_bytes(bytes.try_into().ok()?)),
        _ => None,
    }
}

/// The memory available now, bytes ([`windows_available`]).
#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
pub(crate) fn available() -> Option<f64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX { dwLength: u32::try_from(std::mem::size_of::<MEMORYSTATUSEX>()).ok()?, ..Default::default() };
    // SAFETY: `status` is a writable MEMORYSTATUSEX whose dwLength is its own size, as the call
    // requires; the call fills it and returns 0 on failure. windows-sys has no safe wrapper.
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    if ok == 0 {
        return None;
    }
    windows_available(status.ullAvailPhys, status.ullAvailPageFile)
}

/// The memory available now, bytes ([`mac_available`]).
#[cfg(target_os = "macos")]
pub(crate) fn available() -> Option<f64> {
    mac_available(sysctl(c"hw.memsize")?, sysctl(c"kern.memorystatus_level")?)
}

/// The number `sysctlbyname` gives for `name` ([`sysctl_number`]).
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn sysctl(name: &std::ffi::CStr) -> Option<u64> {
    let mut buf = [0u8; 8];
    let mut len: libc::size_t = buf.len();
    // SAFETY: `name` is NUL-terminated; `buf` is writable for `len` bytes, the most the call writes,
    // and it sets `len` to the bytes written; no new value is given (null, 0). It returns 0 on
    // success. libc has no safe wrapper.
    let status = unsafe { libc::sysctlbyname(name.as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) };
    if status != 0 {
        return None;
    }
    sysctl_number(buf.get(..len)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decision 22: each system's numbers read as the memory available.
    /// Input: Windows's `GlobalMemoryStatusEx` with 9.6 GB available and 30 GB of commit left, then
    /// 3 GB of commit left, then zeros; macOS's `sysctlbyname` bytes for a 16 GiB `hw.memsize` (8
    /// bytes) and a 37 % `kern.memorystatus_level` (4 bytes), readings of other lengths, and shares
    /// of 0 and 101 %. No system is asked. Action: `windows_available`, `sysctl_number`,
    /// `mac_available`. Pass: Windows 9.6 GB, 3 GB with the commit short, the physical memory alone
    /// with no commit reading, nothing without a physical reading; macOS 16 GiB and 37 read back from
    /// their bytes and 37 % of 16 GiB available, nothing for other lengths or shares.
    #[test]
    fn each_systems_numbers_read_as_the_memory_available() {
        assert_eq!(windows_available(9_600_000_000, 30_000_000_000), Some(9_600_000_000.0));
        assert_eq!(windows_available(9_600_000_000, 3_000_000_000), Some(3_000_000_000.0), "the commit is short");
        assert_eq!(windows_available(9_600_000_000, 0), Some(9_600_000_000.0), "no commit reading");
        assert_eq!(windows_available(0, 30_000_000_000), None);
        assert_eq!(windows_available(0, 0), None);
        let gib16: u64 = 16 << 30;
        assert_eq!(sysctl_number(&gib16.to_ne_bytes()), Some(gib16));
        assert_eq!(sysctl_number(&37u32.to_ne_bytes()), Some(37));
        for junk in [&[][..], &[1, 2, 3][..], &[0; 16][..]] {
            assert_eq!(sysctl_number(junk), None, "{junk:?}");
        }
        assert_eq!(mac_available(gib16, 37), Some(gib16 as f64 * 37.0 / 100.0));
        for (memsize, percent) in [(gib16, 0), (gib16, 101), (0, 37)] {
            assert_eq!(mac_available(memsize, percent), None, "{memsize} {percent}");
        }
    }

    /// Decision 22, on Windows and macOS: this computer's memory is read.
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    #[test]
    fn this_computers_memory_is_read() {
        let read = available();
        eprintln!("available here: {read:?}");
        assert!(read.is_some_and(|b| b > 0.0), "{read:?}");
    }
}

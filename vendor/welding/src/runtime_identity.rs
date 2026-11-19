// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0
// SolidCraft: owned, checked identity at the native runtime boundary.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeIdentity {
    pub cef: [u32; 3],
    pub chromium: [u32; 4],
    pub api_version: i32,
    pub platform_api_hash: String,
    pub cef_commit: String,
}

impl RuntimeIdentity {
    pub fn from_parts(
        versions: [i32; 8],
        api_version: i32,
        platform_api_hash: &str,
        cef_commit: &str,
    ) -> Result<Self, &'static str> {
        let mut checked = [0_u32; 8];
        for (output, input) in checked.iter_mut().zip(versions) {
            *output = u32::try_from(input).map_err(|_| "negative CEF version component")?;
        }
        if api_version <= 0 {
            return Err("missing CEF API version");
        }
        for hash in [platform_api_hash, cef_commit] {
            if hash.len() != 40 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err("missing or malformed CEF identity hash");
            }
        }
        let [
            major,
            minor,
            patch,
            _commit_number,
            chrome_major,
            chrome_minor,
            build,
            patch_level,
        ] = checked;
        Ok(Self {
            cef: [major, minor, patch],
            chromium: [chrome_major, chrome_minor, build, patch_level],
            api_version,
            platform_api_hash: platform_api_hash.into(),
            cef_commit: cef_commit.into(),
        })
    }
}

// SolidCraft: the only manual declarations and C-string reads for identity
// inspection live here. Direct-source tests do not enable this native boundary.
#[cfg(all(target_os = "linux", feature = "cef-runtime"))]
mod native {
    use super::RuntimeIdentity;
    use std::ffi::{CStr, c_char, c_int};

    fn version_query() -> Result<unsafe extern "C" fn(c_int) -> c_int, &'static str> {
        // SAFETY: query the already-linked runtime's process-global export;
        // this does not open a library or acquire any CEF shutdown ownership.
        let pointer = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"cef_version_info".as_ptr()) };
        if pointer.is_null() {
            return Err("missing CEF version export");
        }
        // SAFETY: CEF documents this export as int cef_version_info(int).
        // Linux dlsym returns a callable address for a function export. The
        // linked runtime stays loaded; the pointer is used only in inspect.
        Ok(unsafe {
            std::mem::transmute::<*mut std::ffi::c_void, unsafe extern "C" fn(c_int) -> c_int>(
                pointer,
            )
        })
    }

    // Only called with CEF's documented static, NUL-terminated exports.
    fn owned_hash(pointer: *const c_char) -> Result<String, &'static str> {
        if pointer.is_null() {
            return Err("missing CEF identity export");
        }
        // SAFETY: cef_api_hash returns a static NUL-terminated string or null;
        // null is handled above. Copy before returning; retain no native pointer.
        let text = unsafe { CStr::from_ptr(pointer) };
        text.to_str()
            .map(str::to_owned)
            .map_err(|_| "non-UTF-8 CEF identity export")
    }

    pub(super) fn inspect() -> Result<RuntimeIdentity, &'static str> {
        let cef_version_info = version_query()?;
        let mut versions = [0; 8];
        for (entry, value) in (0_i32..=7).zip(versions.iter_mut()) {
            // SAFETY: CEF exports cef_version_info(int); entries 0..=7 are
            // documented scalar queries, available before cef_initialize.
            *value = unsafe { cef_version_info(entry) };
        }
        let platform = owned_hash(cef::api_hash(15400, 0))?;
        let commit = owned_hash(cef::api_hash(15400, 2))?;
        RuntimeIdentity::from_parts(versions, cef::api_version(), &platform, &commit)
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn missing_native_identity_is_recoverable() {
            assert_eq!(
                super::owned_hash(std::ptr::null()),
                Err("missing CEF identity export")
            );
        }
    }
}

#[cfg(all(target_os = "linux", feature = "cef-runtime"))]
pub(crate) fn inspect_native() -> Result<RuntimeIdentity, &'static str> {
    native::inspect()
}

#[cfg(test)]
mod tests {
    use super::*;
    const HASH: &str = "5c8344ae7d7f6421bc801401e35d4dbf6d38e101";
    const COMMIT: &str = "14c5a089e8452874cb1a556dc3fde26ed2d87723";

    #[test]
    fn exported_versions_skip_commit_number() {
        let id =
            RuntimeIdentity::from_parts([154, 0, 34, 3633, 154, 0, 8037, 98], 15400, HASH, COMMIT)
                .unwrap();
        assert_eq!(id.cef, [154, 0, 34]);
        assert_eq!(id.chromium, [154, 0, 8037, 98]);
    }

    #[test]
    fn missing_or_malformed_identity_is_refused() {
        for index in 0..8 {
            let mut versions = [154, 0, 34, 3633, 154, 0, 8037, 98];
            versions[index] = -1;
            assert!(RuntimeIdentity::from_parts(versions, 15400, HASH, COMMIT).is_err());
        }
        for hash in [
            "",
            "abc",
            "zz8344ae7d7f6421bc801401e35d4dbf6d38e101",
            "é8344ae7d7f6421bc801401e35d4dbf6d38e101",
        ] {
            assert!(RuntimeIdentity::from_parts([0; 8], 15400, hash, COMMIT).is_err());
            assert!(RuntimeIdentity::from_parts([0; 8], 15400, HASH, hash).is_err());
        }
        for api in [-1, 0] {
            assert!(RuntimeIdentity::from_parts([0; 8], api, HASH, COMMIT).is_err());
        }
    }
}

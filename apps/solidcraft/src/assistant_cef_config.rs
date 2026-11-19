//! Owned, std-only pre-initialization policy for the Assistant's OSR runtime.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostBackend {
    LinuxWayland,
    LinuxX11,
    Windows,
    Macos,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Switch {
    pub name: String,
    pub value: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeIdentity {
    pub cef: [u32; 3],
    pub chromium: [u32; 4],
    pub api_version: i32,
    pub platform_api_hash: String,
    pub cef_commit: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectivePolicy {
    pub backend: HostBackend,
    pub osr_only: bool,
    pub identity: Option<RuntimeIdentity>,
    pub switches: Vec<Switch>,
}

impl EffectivePolicy {
    /// Whether CEF must ignore the browser process's inherited argv and use
    /// only the approved OnBeforeCommandLineProcessing switches.
    pub fn command_line_args_disabled(&self) -> bool {
        self.backend == HostBackend::LinuxWayland && self.osr_only
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyError {
    InvalidSwitch,
    FeatureConflict,
    OzoneConflict,
    UnknownRuntime,
    InvalidScope,
    AlreadyInitialized,
    ConflictingRuntime,
}

pub fn parse_overrides(raw: &str) -> Result<Vec<Switch>, PolicyError> {
    if raw.len() > 16 * 1024 {
        return Err(PolicyError::InvalidSwitch);
    }
    let mut switches = Vec::new();
    // Legacy syntax is comma-delimited switches, NOT comma-delimited feature
    // values. A bare CamelCase token is rejected instead of losing a feature.
    for part in raw.split(',').filter(|part| !part.is_empty()) {
        let (name, value) = match part.split_once('=') {
            Some((name, value)) => (name, Some(value.to_owned())),
            None => (part, None),
        };
        let entry = Switch { name: name.to_owned(), value };
        validate_switch(&entry)?;
        switches.push(entry);
        if switches.len() > 64 {
            return Err(PolicyError::InvalidSwitch);
        }
    }
    Ok(switches)
}

pub fn build_policy(
    host: HostBackend,
    osr_only: bool,
    identity: Option<&RuntimeIdentity>,
    overrides: &[Switch],
) -> Result<EffectivePolicy, PolicyError> {
    if host == HostBackend::Unknown || (host == HostBackend::LinuxWayland && !osr_only) {
        return Err(PolicyError::InvalidScope);
    }
    let mut switches = vec![switch("no-first-run", None), switch("no-default-browser-check", None), switch("disable-features", Some("Translate"))];
    if host != HostBackend::LinuxWayland {
        // Deliberately preserve the existing non-B defaults, ordering and
        // override semantics. Only the Wayland OSR path uses strict parsing.
        switches.extend_from_slice(overrides);
        return Ok(EffectivePolicy { backend: host, osr_only, identity: identity.cloned(), switches });
    }
    let known = identity.is_some_and(|id| {
        id.cef == [154, 0, 34]
            && id.chromium == [154, 0, 8037, 98]
            && id.api_version == 15400
            && id.platform_api_hash == "5c8344ae7d7f6421bc801401e35d4dbf6d38e101"
            && id.cef_commit == "14c5a089e8452874cb1a556dc3fde26ed2d87723"
    });
    if !known {
        return Err(PolicyError::UnknownRuntime);
    }
    if overrides.len() > 64 {
        return Err(PolicyError::InvalidSwitch);
    }
    let mut disabled = vec!["Translate".to_owned(), "WaylandFractionalScaleV1".to_owned()];
    let mut enabled = Vec::new();
    for entry in overrides {
        validate_switch(entry)?;
        match entry.name.as_str() {
            "disable-features" | "enable-features" => {
                let value = entry.value.as_deref().ok_or(PolicyError::InvalidSwitch)?;
                for feature in value.split(',') {
                    if feature.is_empty() || !feature.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-.:/<>".contains(&b)) {
                        return Err(PolicyError::InvalidSwitch);
                    }
                    let base = feature.split(['<', ':']).next().ok_or(PolicyError::InvalidSwitch)?;
                    if base.is_empty() {
                        return Err(PolicyError::InvalidSwitch);
                    }
                    if entry.name == "enable-features" && base == "WaylandFractionalScaleV1" {
                        return Err(PolicyError::FeatureConflict);
                    }
                    // Disabled names have no trial/parameter syntax. Refuse
                    // ambiguous decorated names rather than mis-merge them.
                    if entry.name == "disable-features" && base != feature {
                        return Err(PolicyError::InvalidSwitch);
                    }
                    let names = if entry.name == "disable-features" { &mut disabled } else { &mut enabled };
                    if !names.iter().any(|name| name == feature) {
                        names.push(feature.to_owned());
                        if names.iter().map(String::len).sum::<usize>() + names.len().saturating_sub(1) > 1024 {
                            return Err(PolicyError::InvalidSwitch);
                        }
                    }
                }
            }
            "ozone-platform" | "ozone-platform-hint" => {
                if entry.value.as_deref() != Some("wayland") {
                    return Err(PolicyError::OzoneConflict);
                }
            }
            "headless" => return Err(PolicyError::OzoneConflict),
            _ => switches.push(entry.clone()),
        }
    }
    switches.retain(|entry| entry.name != "disable-features");
    switches.push(switch("ozone-platform", Some("wayland")));
    switches.push(switch("disable-features", Some(&disabled.join(","))));
    if !enabled.is_empty() {
        switches.push(switch("enable-features", Some(&enabled.join(","))));
    }
    for entry in &switches {
        validate_switch(entry)?;
    }
    Ok(EffectivePolicy { backend: host, osr_only, identity: identity.cloned(), switches })
}

pub fn claim_runtime(slot: &std::sync::OnceLock<EffectivePolicy>, policy: &EffectivePolicy) -> Result<(), PolicyError> {
    match slot.set(policy.clone()) {
        Ok(()) => Ok(()),
        Err(_) if slot.get() == Some(policy) => Err(PolicyError::AlreadyInitialized),
        Err(_) => Err(PolicyError::ConflictingRuntime),
    }
}

fn validate_switch(entry: &Switch) -> Result<(), PolicyError> {
    if entry.name.is_empty()
        || entry.name.len() > 1024
        || !entry.name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || entry.name.starts_with('-')
        || entry.value.as_ref().is_some_and(|v| v.len() > 1024 || v.chars().any(char::is_control))
        || entry.name.starts_with("remote-debugging")
        || entry.name == "auto-open-devtools-for-tabs"
    {
        return Err(PolicyError::InvalidSwitch);
    }
    Ok(())
}

fn switch(name: &str, value: Option<&str>) -> Switch {
    Switch { name: name.into(), value: value.map(str::to_owned) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> RuntimeIdentity {
        RuntimeIdentity {
            cef: [154, 0, 34],
            chromium: [154, 0, 8037, 98],
            api_version: 15400,
            platform_api_hash: "5c8344ae7d7f6421bc801401e35d4dbf6d38e101".into(),
            cef_commit: "14c5a089e8452874cb1a556dc3fde26ed2d87723".into(),
        }
    }

    fn wayland(overrides: &[Switch]) -> Result<EffectivePolicy, PolicyError> {
        build_policy(HostBackend::LinuxWayland, true, Some(&identity()), overrides)
    }

    #[test]
    fn inherited_browser_arguments_are_disabled_only_for_wayland_osr() {
        // Empty environment overrides do not make the browser's argv trusted.
        // CEF must suppress argv even when our approved switch list is empty
        // apart from the defaults; subprocess dispatch is a separate entrypoint.
        assert!(wayland(&[]).unwrap().command_line_args_disabled());
        for host in [HostBackend::LinuxX11, HostBackend::Windows, HostBackend::Macos] {
            assert!(!build_policy(host, true, None, &[]).unwrap().command_line_args_disabled());
        }
    }

    #[test]
    fn merged_features_preserve_translate() {
        let policy = wayland(&[]).unwrap();
        assert_eq!(
            policy.switches.iter().filter(|s| s.name == "disable-features").collect::<Vec<_>>(),
            vec![&switch("disable-features", Some("Translate,WaylandFractionalScaleV1"))]
        );
        let policy = wayland(&[
            switch("disable-features", Some("Translate,ExtraFeature,WaylandFractionalScaleV1")),
            switch("disable-features", Some("ExtraFeature")),
        ])
        .unwrap();
        assert_eq!(
            policy.switches.iter().filter(|s| s.name == "disable-features").collect::<Vec<_>>(),
            vec![&switch("disable-features", Some("Translate,WaylandFractionalScaleV1,ExtraFeature"))]
        );
        for host in [HostBackend::LinuxX11, HostBackend::Windows, HostBackend::Macos] {
            let overrides = [switch("disable-features", Some("Legacy")), switch("MixedCaseLegacy", None)];
            let policy = build_policy(host, true, None, &overrides).unwrap();
            assert_eq!(
                policy.switches,
                vec![
                    switch("no-first-run", None),
                    switch("no-default-browser-check", None),
                    switch("disable-features", Some("Translate")),
                    overrides[0].clone(),
                    overrides[1].clone()
                ]
            );
        }
    }

    #[test]
    fn feature_enable_trial_conflicts() {
        for value in [
            "WaylandFractionalScaleV1",
            "WaylandFractionalScaleV1<Trial",
            "Other,WaylandFractionalScaleV1:parameter/value",
            "WaylandFractionalScaleV1<Trial:parameter/value",
        ] {
            assert_eq!(wayland(&[switch("enable-features", Some(value))]), Err(PolicyError::FeatureConflict));
        }
        assert!(wayland(&[switch("enable-features", Some("Other<Trial:param/value"))]).is_ok());
        for value in ["x11", "headless", ""] {
            assert_eq!(wayland(&[switch("ozone-platform", Some(value))]), Err(PolicyError::OzoneConflict));
        }
        assert_eq!(wayland(&[switch("ozone-platform", Some("wayland")), switch("ozone-platform", Some("x11"))]), Err(PolicyError::OzoneConflict));
        assert_eq!(wayland(&[switch("headless", None)]), Err(PolicyError::OzoneConflict));
        let policy = wayland(&[switch("ozone-platform", Some("wayland"))]).unwrap();
        assert_eq!(policy.switches.iter().filter(|s| s.name == "ozone-platform").count(), 1);
    }

    #[test]
    fn backend_identity_and_claims_are_scoped() {
        assert_eq!(build_policy(HostBackend::Unknown, true, Some(&identity()), &[]), Err(PolicyError::InvalidScope));
        assert_eq!(build_policy(HostBackend::LinuxWayland, false, Some(&identity()), &[]), Err(PolicyError::InvalidScope));
        assert_eq!(build_policy(HostBackend::LinuxWayland, true, None, &[]), Err(PolicyError::UnknownRuntime));
        let mut unknown = identity();
        unknown.chromium[3] += 1;
        assert_eq!(build_policy(HostBackend::LinuxWayland, true, Some(&unknown), &[]), Err(PolicyError::UnknownRuntime));
        let policy = wayland(&[]).unwrap();
        let slot = std::sync::OnceLock::new();
        assert_eq!(claim_runtime(&slot, &policy), Ok(()));
        assert_eq!(claim_runtime(&slot, &policy), Err(PolicyError::AlreadyInitialized));
        let mut different = policy.clone();
        different.identity = Some(unknown);
        assert_eq!(claim_runtime(&slot, &different), Err(PolicyError::ConflictingRuntime));
        different = policy.clone();
        different.switches.push(switch("test", None));
        assert_eq!(claim_runtime(&slot, &different), Err(PolicyError::ConflictingRuntime));
    }

    #[test]
    fn diagnostic_overrides_are_bounded() {
        for raw in [
            "remote-debugging-port=9222",
            "remote-debugging-address=0.0.0.0",
            "auto-open-devtools-for-tabs",
            "disable-features=Translate,WaylandFractionalScaleV1",
            "BadName=x",
            "--headless",
            "white space=x",
            "name=\nsecret",
            "=x",
        ] {
            assert_eq!(parse_overrides(raw).and_then(|v| wayland(&v)), Err(PolicyError::InvalidSwitch), "{raw:?}");
        }
        assert!(parse_overrides("disable-features=ExtraFeature,enable-logging=stderr").and_then(|v| wayland(&v)).is_ok());
        assert!(parse_overrides(&format!("test={}", "a".repeat(1024))).is_ok());
        assert_eq!(parse_overrides(&format!("test={}", "a".repeat(1025))), Err(PolicyError::InvalidSwitch));
        assert!(parse_overrides(&vec!["test"; 64].join(",")).is_ok());
        assert_eq!(parse_overrides(&vec!["test"; 65].join(",")), Err(PolicyError::InvalidSwitch));
        assert_eq!(parse_overrides(&"a".repeat(16 * 1024 + 1)), Err(PolicyError::InvalidSwitch));
        assert_eq!(wayland(&[switch("remote-debugging-port", Some("0"))]), Err(PolicyError::InvalidSwitch));
        assert_eq!(wayland(&[switch("disable-features", Some(&"a".repeat(1025)))]), Err(PolicyError::InvalidSwitch));
        // Bounds also apply after structural merging, not only to each input.
        assert_eq!(
            wayland(&[switch("disable-features", Some(&"a".repeat(600))), switch("disable-features", Some(&"b".repeat(600))),]),
            Err(PolicyError::InvalidSwitch)
        );
    }
}

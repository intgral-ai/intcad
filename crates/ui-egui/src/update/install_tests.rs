use super::install::*;
use super::{Asset, FetchError, InstallKind, Release};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

// Independent minisign test vectors (Ed25519 over BLAKE2b-512, public fixture seed 00..1f).
// No production key or external signing tool is needed to run these tests.
pub(super) const TEST_KEY: &str = "RWRmaXh0dXJlIQOhB7/zzhC+HXDdGOdLwJln5NYwm6UNXx3chmQSVTG4";
const APPIMAGE: &str = "intcad-linux-x86_64.AppImage";
const NEW: &[u8] = b"new version";
const SIGNED_SUMS: &str = "30e4af48343ed4e5b11ac1d072de79d7b02107894975ceda841cce472a63ab33  intcad-linux-x86_64.AppImage\n";
const SUMS_LINE: &str = "RURmaXh0dXJlIWyqjIneDROGCT0SgknSslWgDxg14/X0KvehB+3n0ldRsH7i5msy+DAwXXWRtT7UC6bWPuDnwYNOWBgBw1jLugs=";

fn signature(line: &str, comment: &str, global: &str) -> String {
    format!("untrusted comment: fixture\n{line}\ntrusted comment: {comment}\n{global}\n")
}

fn appimage_signature() -> String {
    signature(SUMS_LINE, "Intcad v9.9.9", "NW963dVMWpzY9Xvk2lDywVkqdzA15wSnAeihCSA0mlYKZ9x4GQ4Jchoesi/o860+cr4RYPxi2/aKLp0aPvt3Dw==")
}

pub(super) const UI_SUMS: &str = concat!(
    "30e4af48343ed4e5b11ac1d072de79d7b02107894975ceda841cce472a63ab33  intcad-linux-x86_64.AppImage\n",
    "30e4af48343ed4e5b11ac1d072de79d7b02107894975ceda841cce472a63ab33  intcad-linux-aarch64.AppImage\n",
);

pub(super) fn ui_signature() -> String {
    signature(
        "RURmaXh0dXJlIWuBu2ZZRWCqgC3aq6qWy06dpDtp29Zw5JUNkc9i5sWGpMX5WQAOdg9SpcXYzZg9mZtjbtkQXqiW7g28IJ7rPAs=",
        "Intcad v999.1.0",
        "4jlVMtNob7n14jmnVQuk4ZsWLWoDifXP37BSsjKSbCwNfId7oJPvssLmXu5kMvHkhK+ccX1jZRAhSNaLmKmWCA==",
    )
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("solidcraft-update-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn checksum_lists_are_read() {
    let h = "a".repeat(64);
    let sums = format!("{h}  intcad-linux-x86_64.AppImage\n{}  *intcad-windows-x64.msi\nnot a line\n{}  short.txt\n", "B".repeat(64), "c".repeat(10));
    assert_eq!(expected_sha256(&sums, "intcad-linux-x86_64.AppImage"), Some(h));
    assert_eq!(expected_sha256(&sums, "intcad-windows-x64.msi"), Some("b".repeat(64)), "binary-mode marker and upper case");
    assert_eq!(expected_sha256(&sums, "short.txt"), None, "not a SHA-256");
    assert_eq!(expected_sha256(&sums, "intcad-linux-x86_64"), None);
}

#[test]
fn signatures_must_match_the_key_the_list_and_the_release() {
    let signature = appimage_signature();
    let key = minisign_verify::PublicKey::from_base64(TEST_KEY).unwrap();
    assert_eq!(verify_signature(SIGNED_SUMS.as_bytes(), &signature, &key, "v9.9.9"), Ok(()));
    assert!(verify_signature(b"bbbb  intcad-linux-x86_64.AppImage\n", &signature, &key, "v9.9.9").is_err(), "a changed list");
    // Same key id as the trusted fixture key, but different public key bytes.
    let other = minisign_verify::PublicKey::from_base64("RWRmaXh0dXJlIXmlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3").unwrap();
    assert!(verify_signature(SIGNED_SUMS.as_bytes(), &signature, &other, "v9.9.9").is_err(), "another key");
    assert!(verify_signature(SIGNED_SUMS.as_bytes(), "garbage", &key, "v9.9.9").is_err());
}

#[test]
fn a_valid_signature_requires_the_exact_trusted_comment_and_a_stable_v_tag() {
    for (tag, comment, global) in [
        ("v9.9.9", "not Intcad v9.9.9", "hyylxS4gj1oERcgkhgyNBv9LAWHcEF6Ky4oMmpdnL6JhoKISCNu8SSo7g9ivWFqik9dAFqMTXwqoFAuBEKmkBA=="),
        ("v9.9.9", "SolidCraft v9.9.9", "25TRDxoBLltBMDlyKo7l5oiRCb0i+2bg795ycmNCioaM2WErrUcgo3Ta/uMIpPFgO48/FD6azs33jzctnC4HBA=="),
        ("v9.9.9-rc.1", "Intcad v9.9.9-rc.1", "doszLlgYnmhnsdavb3cx4RnBtpZ07m6ch67uTLbPzH6yjbp1lGFdz1s/AEpV5XF5QeE56wlJ71GLs/NmqUA/BA=="),
        ("9.9.9", "Intcad 9.9.9", "93xkDpyEd4tB1Rmr3n5n0zSuWpDx7wSciYkc50eDQr3xoHvGdiCjACc7WTCe0OQwFeLbIuqULP+YM47Bvm9qAw=="),
        ("v9.9.9+build.1", "Intcad v9.9.9+build.1", "2YFOgOJibmmqocWOeZvDw2fOCWcY5Le56d1xwCCIUfVxqSBwMjH+9bsiLhS3sr6MhNKca68YYzkhWJfkzakqBA=="),
    ] {
        let sig = signature(SUMS_LINE, comment, global);
        let key = minisign_verify::PublicKey::from_base64(TEST_KEY).unwrap();
        key.verify(SIGNED_SUMS.as_bytes(), &minisign_verify::Signature::decode(&sig).unwrap(), false).unwrap();
        assert!(verify_signature(SIGNED_SUMS.as_bytes(), &sig, &key, tag).is_err(), "{comment}");
    }
}

#[test]
fn methods_per_installation() {
    let exe = Path::new("/x/intcad.exe");
    assert_eq!(method(&InstallKind::AppImage("/a/S.AppImage".into()), Some(exe)), Some(Method::AppImage("/a/S.AppImage".into())));
    assert_eq!(method(&InstallKind::WindowsMsi, Some(exe)), Some(Method::Msi));
    assert_eq!(method(&InstallKind::WindowsPortable, Some(exe)), Some(Method::PortableZip(exe.into())));
    assert_eq!(method(&InstallKind::WindowsExe, Some(exe)), Some(Method::Exe(exe.into())));
    assert_eq!(method(&InstallKind::MacApp("/A/S.app".into()), Some(exe)), Some(Method::Dmg));
    for never in [
        InstallKind::Flatpak("id".into()),
        InstallKind::Snap { store: true },
        InstallKind::Snap { store: false },
        InstallKind::Deb,
        InstallKind::Rpm,
        InstallKind::LinuxTarball,
        InstallKind::Other,
        InstallKind::Dev,
    ] {
        assert_eq!(method(&never, Some(exe)), None, "{never:?} is never replaced in place");
    }
}

const BASE: &str = "https://github.com/intgral-ai/intcad-releases/releases/download/v9.9.9/";

fn release_with(assets: &[(&str, usize)]) -> Release {
    Release {
        tag: "v9.9.9".into(),
        name: "SolidCraft v9.9.9".into(),
        notes: String::new(),
        page: "https://github.com/intgral-ai/intcad-releases/releases/tag/v9.9.9".into(),
        assets: assets.iter().map(|(n, size)| Asset { name: n.to_string(), url: format!("{BASE}{n}"), size: *size as u64 }).collect(),
    }
}

/// A network that serves `files` by name.
fn net(files: Vec<(String, Vec<u8>)>) -> Net {
    let files = Arc::new(files);
    let f2 = files.clone();
    let find = move |files: &[(String, Vec<u8>)], url: &str| files.iter().find(|(n, _)| url == format!("{BASE}{n}")).map(|(_, b)| b.clone());
    let find2 = find;
    Net {
        feed: None,
        test_key: Some(TEST_KEY),
        get: Arc::new(move |url: &str| find(&files, url).ok_or_else(|| FetchError::Failed(format!("404 {url}")))),
        download: Arc::new(move |url: &str, to: &Path, progress: &dyn Fn(u64, Option<u64>) -> bool| {
            let body = find2(&f2, url).ok_or_else(|| FetchError::Failed(format!("404 {url}")))?;
            let mut file = std::fs::File::create(to).map_err(|e| FetchError::Failed(e.to_string()))?;
            let mut done = 0;
            for chunk in body.chunks(4) {
                file.write_all(chunk).map_err(|e| FetchError::Failed(e.to_string()))?;
                done += chunk.len() as u64;
                if !progress(done, Some(body.len() as u64)) {
                    return Err(FetchError::Failed("cancelled".into()));
                }
            }
            Ok(())
        }),
    }
}

#[test]
fn the_built_in_trust_path_rejects_an_unsigned_release() {
    let dir = scratch("built-in-key");
    let target = dir.join("Intcad.AppImage");
    std::fs::write(&target, b"old version").unwrap();
    let name = "intcad-linux-x86_64.AppImage";
    let content = b"new version";
    let kind = InstallKind::AppImage(target.clone());
    let m = Method::AppImage(target.clone());
    for sums_name in [SUMS, SUMS_LEGACY] {
        let r = release_with(&[(name, content.len()), (sums_name, 100)]);
        let sums = format!("{}  {name}\n", hex(content));
        let mut n = net(vec![(name.into(), content.to_vec()), (sums_name.into(), sums.into_bytes())]);
        n.test_key = None;
        n.download = Arc::new(|_, _, _| panic!("unsigned package must not be downloaded"));
        assert!(matches!(run(&r, &kind, &m, "x86_64", &n, &dir, &Progress::default()), Err(Error::Verification(_))), "{sums_name}");
        assert_eq!(std::fs::read(&target).unwrap(), b"old version");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_invalid_release_key_rejects_even_a_signed_list() {
    let dir = scratch("invalid-key");
    let target = dir.join("Intcad.AppImage");
    std::fs::write(&target, b"old version").unwrap();
    let r = release_with(&[(APPIMAGE, NEW.len()), (SUMS, SIGNED_SUMS.len()), (SUMS_SIG, 100)]);
    let mut n = net(vec![(APPIMAGE.into(), NEW.to_vec()), (SUMS.into(), SIGNED_SUMS.into()), (SUMS_SIG.into(), appimage_signature().into_bytes())]);
    n.test_key = Some("not a public key");
    n.download = Arc::new(|_, _, _| panic!("invalid key must prevent the package download"));
    let result = run(&r, &InstallKind::AppImage(target.clone()), &Method::AppImage(target.clone()), "x86_64", &n, &dir, &Progress::default());
    assert!(matches!(result, Err(Error::Verification(detail)) if detail.contains("update signing key")));
    assert_eq!(std::fs::read(&target).unwrap(), b"old version");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_signed_release_can_use_the_legacy_checksum_name() {
    let dir = scratch("legacy-sums");
    let target = dir.join("Intcad.AppImage");
    std::fs::write(&target, b"old version").unwrap();
    let kind = InstallKind::AppImage(target.clone());
    let m = Method::AppImage(target.clone());
    let r = release_with(&[(APPIMAGE, NEW.len()), (SUMS_LEGACY, SIGNED_SUMS.len()), (SUMS_SIG, 100)]);

    let n =
        net(vec![(APPIMAGE.into(), NEW.to_vec()), (SUMS_LEGACY.into(), SIGNED_SUMS.into()), (SUMS_SIG.into(), appimage_signature().into_bytes())]);
    let outcome = run(&r, &kind, &m, "x86_64", &n, &dir, &Progress::default()).unwrap();
    assert_eq!(outcome, Outcome::Restart(target.clone()));
    assert_eq!(std::fs::read(&target).unwrap(), NEW);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_appimage_is_replaced_only_when_it_verifies() {
    let dir = scratch("appimage");
    let target = dir.join("SolidCraft.AppImage");
    std::fs::write(&target, b"old version").unwrap();
    let kind = InstallKind::AppImage(target.clone());
    let m = Method::AppImage(target.clone());
    let r = release_with(&[(APPIMAGE, NEW.len()), (SUMS, SIGNED_SUMS.len()), (SUMS_SIG, 100)]);
    let sig = appimage_signature().into_bytes();

    // A package that doesn't match the list: deleted, the old one untouched.
    let n = net(vec![(APPIMAGE.into(), b"bad version".to_vec()), (SUMS.into(), SIGNED_SUMS.into()), (SUMS_SIG.into(), sig.clone())]);
    let err = run(&r, &kind, &m, "x86_64", &n, &dir.join("staging"), &Progress::default()).unwrap_err();
    assert!(matches!(err, Error::Verification(ref detail) if detail.contains("doesn't match")), "{err}");
    assert_eq!(std::fs::read(&target).unwrap(), b"old version");
    assert!(!dir.join("SolidCraft.AppImage.update").exists(), "the bad download is deleted");

    // Not listed at all.
    let empty_sig = signature(
        "RURmaXh0dXJlISYKHljc2XpaJTda2dqoPrAxHgNW4rDJZOuE+pNHxtpaO/Q9MTJAr86S+jhkA/fHkCm8KJvH/PJaimm3w6272gk=",
        "Intcad v9.9.9",
        "DyxA6s9Qr/JTfx5C9wY8quPM5pE7K798eFTr/F46Dv7NnTWIkcd0bnyB9I/ulpK7uTQC8bEj6uggrJ1ZoC3RCA==",
    );
    let n = net(vec![(SUMS.into(), vec![]), (SUMS_SIG.into(), empty_sig.into_bytes())]);
    assert!(run(&r, &kind, &m, "x86_64", &n, &dir, &Progress::default()).unwrap_err().to_string().contains("doesn't list"));
    // No checksum list in the release: refused before downloading.
    let bare = release_with(&[(APPIMAGE, NEW.len())]);
    assert!(run(&bare, &kind, &m, "x86_64", &n, &dir, &Progress::default()).unwrap_err().to_string().contains(SUMS));

    // Cancelled before any network work: nothing changes.
    let p = Progress::default();
    p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    let err = run(&r, &kind, &m, "x86_64", &n, &dir, &p).unwrap_err();
    assert!(matches!(err, Error::Cancelled), "{err}");
    assert_eq!(std::fs::read(&target).unwrap(), b"old version");

    // Cancel during the checksum GET: package download never starts (#8908).
    let progress = Arc::new(Progress::default());
    let flag = progress.clone();
    let downloaded = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hit = downloaded.clone();
    let mut n = net(vec![(APPIMAGE.into(), NEW.to_vec()), (SUMS.into(), SIGNED_SUMS.into()), (SUMS_SIG.into(), sig)]);
    let get = n.get.clone();
    n.get = Arc::new(move |url| {
        let response = get(url);
        if url.ends_with(SUMS) {
            flag.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        response
    });
    let download = n.download.clone();
    n.download = Arc::new(move |url, to, report| {
        hit.store(true, std::sync::atomic::Ordering::Relaxed);
        download(url, to, report)
    });
    let err = run(&r, &kind, &m, "x86_64", &n, &dir, &progress).unwrap_err();
    assert!(matches!(err, Error::Cancelled), "{err}");
    assert!(!downloaded.load(std::sync::atomic::Ordering::Relaxed), "package must not start after Cancel");
    assert_eq!(std::fs::read(&target).unwrap(), b"old version");

    // Verified: renamed over the old one, executable, restart offered.
    let out = run(&r, &kind, &m, "x86_64", &n, &dir, &Progress::default()).unwrap();
    assert_eq!(out, Outcome::Restart(target.clone()));
    assert_eq!(std::fs::read(&target).unwrap(), NEW);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o755);
    }
    assert_eq!(std::fs::read_dir(&dir).unwrap().filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().ends_with(".update")).count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_portable_zip_swaps_both_executables() {
    let dir = scratch("portable");
    let exe = dir.join("intcad.exe");
    std::fs::write(&exe, b"old app").unwrap();
    std::fs::write(dir.join("intcad-cli.exe"), b"old cli").unwrap();
    let mut zip_bytes = Vec::new();
    {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut zip_bytes));
        let o = zip::write::SimpleFileOptions::default();
        z.add_directory("intcad-9.9.9-windows-x64-portable/", o).unwrap();
        for (n, b) in [("intcad.exe", &b"new app"[..]), ("intcad-cli.exe", b"new cli"), ("README.md", b"readme")] {
            z.start_file(format!("intcad-9.9.9-windows-x64-portable/{n}"), o).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap();
    }
    let name = "intcad-windows-x64-portable.zip";
    // Exercise replacement after verification; the signed worker path is covered by the AppImage tests.
    let package = dir.join(name);
    std::fs::write(&package, &zip_bytes).unwrap();
    let out = apply(&Method::PortableZip(exe.clone()), &package).unwrap();
    assert_eq!(out, Outcome::Restart(exe.clone()));
    assert_eq!(std::fs::read(&exe).unwrap(), b"new app");
    assert_eq!(std::fs::read(dir.join("intcad-cli.exe")).unwrap(), b"new cli");
    assert!(!package.exists(), "the zip is removed");
    // A zip without the app is refused and changes nothing.
    std::fs::write(dir.join("empty.zip"), {
        let mut b = Vec::new();
        zip::ZipWriter::new(std::io::Cursor::new(&mut b)).finish().unwrap();
        b
    })
    .unwrap();
    assert!(apply(&Method::PortableZip(exe.clone()), &dir.join("empty.zip")).is_err());
    assert_eq!(std::fs::read(&exe).unwrap(), b"new app");
    // The moved-aside executable of an update goes at the next start.
    std::fs::write(dir.join("intcad.exe.old"), b"x").unwrap();
    clean_up_after_update(Some(&exe));
    assert!(!dir.join("intcad.exe.old").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn installers_and_disk_images_are_handed_over() {
    let dir = scratch("msi");
    let pkg = dir.join("intcad-windows-x64.msi");
    std::fs::write(&pkg, b"msi").unwrap();
    assert_eq!(
        apply(&Method::Msi, &pkg).unwrap(),
        Outcome::Installer("msiexec".into(), vec!["/i".into(), pkg.to_string_lossy().to_string(), "/passive".into()])
    );
    assert_eq!(apply(&Method::Dmg, &pkg).unwrap(), Outcome::Opened(pkg.clone()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn windows_installer_is_followed_by_the_new_version() {
    let args = vec!["/i".to_string(), r"C:\Users\a b\Temp\solidcraft-update\x.msi".to_string(), "/passive".to_string()];
    let (program, out) = relaunch_after(Path::new("msiexec"), &args, Path::new(r"C:\Program Files\Intcad\it's.exe"));
    assert_eq!(program, PathBuf::from("powershell"));
    assert_eq!(
        out.last().unwrap(),
        &concat!(
            r#"Start-Process -FilePath 'msiexec' -ArgumentList '/i','"C:\Users\a b\Temp\solidcraft-update\x.msi"','/passive' -Wait; "#,
            r"Start-Process -FilePath 'C:\Program Files\Intcad\it''s.exe'"
        )
    );
    assert!(out.iter().any(|a| a == "-Command"));
}

#[test]
fn an_updated_appimage_takes_the_release_file_name() {
    let new = |p: &str| release_named(Path::new(p), "v0.17.0", "x86_64");
    assert_eq!(new("/a/intcad-0.16.2-linux-x86_64.AppImage"), Path::new("/a/intcad-0.17.0-linux-x86_64.AppImage"));
    assert_eq!(new("/a/solvecraft-0.9.0-linux-x86_64.AppImage"), Path::new("/a/intcad-0.17.0-linux-x86_64.AppImage"));
    assert_eq!(new("/a/SolidCraft.AppImage"), Path::new("/a/SolidCraft.AppImage"), "a name the user chose");
    assert_eq!(new("/a/intcad-linux-x86_64.AppImage"), Path::new("/a/intcad-linux-x86_64.AppImage"), "no version in it");
    assert_eq!(new("/a/intcad-0.16.2-linux-x86_64.AppImage.bak"), Path::new("/a/intcad-0.16.2-linux-x86_64.AppImage.bak"));

    // End to end: the old versioned file is gone, the new one has the release's name and works.
    let dir = scratch("appimage-name");
    let old = dir.join("intcad-9.9.8-linux-x86_64.AppImage");
    std::fs::write(&old, b"old version").unwrap();
    let r = release_with(&[(APPIMAGE, NEW.len()), (SUMS, SIGNED_SUMS.len()), (SUMS_SIG, 100)]);
    let n = net(vec![(APPIMAGE.into(), NEW.to_vec()), (SUMS.into(), SIGNED_SUMS.into()), (SUMS_SIG.into(), appimage_signature().into_bytes())]);
    let kind = InstallKind::AppImage(old.clone());
    let out = run(&r, &kind, &Method::AppImage(old.clone()), "x86_64", &n, &dir, &Progress::default()).unwrap();
    let want = dir.join("intcad-9.9.9-linux-x86_64.AppImage");
    assert_eq!(out, Outcome::Restart(want.clone()));
    assert_eq!(std::fs::read(&want).unwrap(), NEW);
    assert!(!old.exists());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "nothing else is left behind");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn installation_refreshes_the_release_and_verifies_the_final_package() {
    let dir = scratch("refresh");
    let target = dir.join("SolidCraft.AppImage");
    let name = "intcad-linux-x86_64.AppImage";
    let shown = release_with(&[(name, 3), (SUMS, 100), (SUMS_SIG, 100)]);
    let base = "https://github.com/intgral-ai/intcad-releases/releases/download/v10.0.0/";
    let feed = serde_json::json!({
        "tag_name": "v10.0.0",
        "assets": [
            {"name": name, "browser_download_url": format!("{base}{name}"), "size": 3},
            {"name": SUMS, "browser_download_url": format!("{base}{SUMS}")},
            {"name": SUMS_SIG, "browser_download_url": format!("{base}{SUMS_SIG}")}
        ]
    })
    .to_string()
    .into_bytes();
    for (case, answer, corrupt, expected, fallback) in [
        ("newest", Ok(feed.clone()), false, "10.0.0", false),
        ("corrupt newest", Ok(feed.clone()), true, "10.0.0", false),
        ("signature for the offered release, not the newest", Ok(feed), false, "10.0.0", false),
        ("offline", Err(FetchError::Failed("offline".into())), false, "9.9.9", true),
        ("rate limited", Err(FetchError::RateLimited { retry_after_s: 60 }), false, "9.9.9", true),
        ("oversized", Ok(vec![b' '; super::MAX_RELEASE_BYTES + 1]), false, "9.9.9", true),
        ("older", Ok(br#"{"tag_name":"v9.9.8"}"#.to_vec()), false, "9.9.9", false),
    ] {
        std::fs::write(&target, b"old").unwrap();
        let chosen_base = if expected == "10.0.0" { base } else { BASE };
        let replayed = case.starts_with("signature");
        let (comment, global) = if chosen_base == base && !replayed {
            ("Intcad v10.0.0", "HSPx0BbjXIe36rv5Sc2AyA0Tekdowyyc9Y4TZLS8WoHE7Tr+bIFkyZLCS30JWcmyy6KQB0rSZS5GEQ4RJRjWCw==")
        } else {
            ("Intcad v9.9.9", "vbW5Ex7ZYVzxnHe3hInQPU0JgcBlazTbqouSGuKTwcKZG3bEUMpN9kaDl7zZeSNU04hnYqgiDig9JjByuAGPCA==")
        };
        let n = Net {
            feed: None,
            test_key: Some(TEST_KEY),
            get: Arc::new(move |url| {
                if url == super::LATEST_API {
                    return answer.clone();
                }
                if url == format!("{chosen_base}{SUMS_SIG}") {
                    return Ok(signature(
                        "RURmaXh0dXJlIUVRVvq7bsUDLEdfMYtuN2uPY0RrQtfappeKj/3TYo0OSUC2R/NVXFUYk70yLSIoj7rtk+7gza5Thc6xl/jLTQQ=",
                        comment,
                        global,
                    )
                    .into_bytes());
                }
                assert_eq!(url, format!("{chosen_base}{SUMS}"), "checksums from the chosen release");
                Ok(format!("{}  {name}\n", hex(b"new")).into_bytes())
            }),
            download: Arc::new(move |url, to, report| {
                assert!(!replayed, "a signature for the previously offered version must not start the download");
                assert_eq!(url, format!("{chosen_base}{name}"), "package from the chosen release");
                std::fs::write(to, if corrupt { b"bad" } else { b"new" }).unwrap();
                assert!(report(3, Some(3)));
                Ok(())
            }),
        };
        let p = Progress::default();
        let result = run(&shown, &InstallKind::AppImage(target.clone()), &Method::AppImage(target.clone()), "x86_64", &n, &dir, &p);
        let (selected, used_fallback) = p.selection().unwrap();
        assert_eq!(selected.version(), expected, "{case}");
        assert_eq!(used_fallback, fallback, "{case}");
        assert_eq!(
            p.message(),
            crate::i18n::format(if fallback { "update.refresh_fallback" } else { "update.refresh_selected" }, &[("version", expected)]),
            "{case}"
        );
        if corrupt || replayed {
            let err = result.unwrap_err();
            let detail = if corrupt { "doesn't match" } else { "does not match v10.0.0" };
            assert!(matches!(err, Error::Verification(ref message) if message.contains(detail)), "{err}");
            assert_eq!(std::fs::read(&target).unwrap(), b"old");
        } else {
            assert_eq!(result.unwrap(), Outcome::Restart(target.clone()), "{case}");
            assert_eq!(std::fs::read(&target).unwrap(), b"new");
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn unauthenticated_lists_stop_the_update_before_the_package_download() {
    let dir = scratch("signed-refusals");
    let target = dir.join("Intcad.AppImage");
    std::fs::write(&target, b"old version").unwrap();
    let kind = InstallKind::AppImage(target.clone());
    let m = Method::AppImage(target.clone());
    let valid_sig = appimage_signature();
    let not_downloaded =
        |_: &str, _: &Path, _: &dyn Fn(u64, Option<u64>) -> bool| panic!("package download must not start without an authentic list");
    for sums_name in [SUMS, SUMS_LEGACY] {
        let r = release_with(&[(APPIMAGE, NEW.len()), (sums_name, SIGNED_SUMS.len())]);
        let mut n = net(vec![(sums_name.into(), SIGNED_SUMS.into())]);
        n.download = Arc::new(not_downloaded);
        let err = run(&r, &kind, &m, "x86_64", &n, &dir, &Progress::default()).unwrap_err();
        assert!(matches!(err, Error::Verification(ref detail) if detail.contains("isn't signed")), "{err}");
    }
    // Use exactly the bytes that are signed for v9.9.9, so only the version check can fail.
    let mut r = release_with(&[(APPIMAGE, NEW.len()), (SUMS, SIGNED_SUMS.len()), (SUMS_SIG, 100)]);
    r.tag = "v9.9.8".into();
    let mut n = net(vec![(SUMS.into(), SIGNED_SUMS.into()), (SUMS_SIG.into(), valid_sig.clone().into_bytes())]);
    n.download = Arc::new(not_downloaded);
    let err = run(&r, &kind, &m, "x86_64", &n, &dir, &Progress::default()).unwrap_err();
    assert!(matches!(err, Error::Verification(ref detail) if detail.contains("does not match v9.9.8")), "{err}");

    // A signed legacy list cannot authorize different bytes under the preferred name.
    let r = release_with(&[(APPIMAGE, NEW.len()), (SUMS, SIGNED_SUMS.len()), (SUMS_LEGACY, SIGNED_SUMS.len()), (SUMS_SIG, 100)]);
    let mut n = net(vec![
        (SUMS.into(), format!("{SIGNED_SUMS}\n").into_bytes()),
        (SUMS_LEGACY.into(), SIGNED_SUMS.into()),
        (SUMS_SIG.into(), valid_sig.into_bytes()),
    ]);
    n.download = Arc::new(not_downloaded);
    let err = run(&r, &kind, &m, "x86_64", &n, &dir, &Progress::default()).unwrap_err();
    assert!(matches!(err, Error::Verification(ref detail) if detail.contains("isn't signed by")), "{err}");
    assert_eq!(std::fs::read(&target).unwrap(), b"old version");
    let _ = std::fs::remove_dir_all(dir);
}

/// The build carries a real production key, not the placeholder: parsing it is the
/// gate that keeps a release from shipping with one-click installs silently dead.
/// Restoring the placeholder makes this test fail on purpose.
#[test]
fn the_embedded_release_key_is_a_configured_minisign_key() {
    assert!(!PUBLIC_KEY.contains("NOT CONFIGURED"), "the placeholder is still in the tree");
    let key = minisign_verify::PublicKey::from_base64(PUBLIC_KEY.trim()).expect("embedded key parses");
    assert_eq!(PUBLIC_KEY.trim().len(), 56, "one base64 line");
    let _ = key;
}

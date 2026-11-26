#![forbid(unsafe_code)]

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons/app/windows/exports/solidcraft-executable.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS")?;
    if target_os == "macos" {
        return macos_account_record();
    }
    if target_os != "windows" {
        return Ok(());
    }
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").ok_or("missing OUT_DIR")?);
    let copy = out.join("solidcraft.ico");
    std::fs::copy(icon, &copy)?;
    let rc = out.join("solidcraft.rc");
    let icon_line = format!("1 ICON \"{}\"\n", copy.display().to_string().replace('\\', "/"));
    std::fs::write(&rc, format!("{icon_line}{}", version_resource(&std::env::var("CARGO_PKG_VERSION")?)))?;
    embed_resource::compile_for(&rc, ["intcad"], embed_resource::NONE).manifest_required()?;
    Ok(())
}

/// One NUL-delimited record for both the runtime constructor and the mounted-DMG verifier.
/// The order and marker match `scripts/ci/verify_embedded_account.py`.
fn macos_account_record() -> Result<(), Box<dyn std::error::Error>> {
    let mut record = String::from("INTCAD_ACCOUNT_CONFIG_V1\0");
    for key in ["INTCAD_WORKOS_PUBLIC_CLIENT_ID", "INTCAD_AUTHKIT_VERIFICATION_ORIGIN", "INTCAD_PRODUCT_HTTPS_ORIGIN"] {
        println!("cargo:rerun-if-env-changed={key}");
        if let Ok(value) = std::env::var(key) {
            record.push_str(&value);
        }
        record.push('\0');
    }
    record.push('\x1e');
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").ok_or("missing OUT_DIR")?);
    std::fs::write(out.join("macos-account-v1"), record)?;
    Ok(())
}

/// The Windows version resource (what Explorer's Details tab shows and what a launcher reads).
/// `FileVersion` and `ProductVersion` are the package version verbatim (`0.25.1`, `0.26.0-rc.1`):
/// ArtCraft's launcher parses the `ProductVersion` string as semver, so a fourth `.0` would not
/// read. The binary `FILEVERSION` fields need four numbers and take the leading `major.minor.patch`.
fn version_resource(version: &str) -> String {
    let mut numbers = version.split(['-', '+']).next().unwrap_or("").split('.').map(|p| p.parse::<u16>().unwrap_or(0));
    let mut next = || numbers.next().unwrap_or(0);
    let (major, minor, patch) = (next(), next(), next());
    format!(
        "1 VERSIONINFO\nFILEVERSION {major},{minor},{patch},0\nPRODUCTVERSION {major},{minor},{patch},0\nFILEOS 0x40004\nFILETYPE 0x1\n{{\nBLOCK \"StringFileInfo\"\n{{\nBLOCK \"040904B0\"\n{{\nVALUE \"CompanyName\", \"Intgral\"\nVALUE \"FileDescription\", \"Intcad, parametric 3D CAD\"\nVALUE \"FileVersion\", \"{version}\"\nVALUE \"InternalName\", \"solidcraft\"\nVALUE \"OriginalFilename\", \"intcad.exe\"\nVALUE \"ProductName\", \"Intcad\"\nVALUE \"ProductVersion\", \"{version}\"\n}}\n}}\nBLOCK \"VarFileInfo\"\n{{\nVALUE \"Translation\", 0x0409, 1200\n}}\n}}\n"
    )
}

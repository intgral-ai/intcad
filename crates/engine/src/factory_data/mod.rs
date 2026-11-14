//! Project factory data: validate one selected file, copy the whole file into the project, and
//! say whether it applies to the declared process, material and thickness.
//!
//! The rule layer owns the types and the validation; this module only reads bytes, hashes them and
//! keeps the project's copy. It never edits the values of a factory file.

use std::fs;
use std::path::{Path, PathBuf};

use solidcraft_dfm_rules::{CheckInput, Declared, Layer, Profile};

use crate::project::{DataKind, FOLDER};

/// The generic layer's identity. The build ships no public shop table yet; a generic profile
/// without requirements means "no covered number", never a guessed one.
pub const GENERIC_NAME: &str = "generic";
pub const GENERIC_VERSION: &str = "1";

/// A whole factory or standard file, validated and copied into a project.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportedData {
    pub kind: DataKind,
    pub name: String,
    pub version: String,
    pub sha256: String,
    /// Project-relative path of the copy, always inside `.intcad/data/`.
    pub file: String,
    pub profile: Profile,
}

/// Why a selected file cannot be used.
#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error("{0}")]
    Invalid(String),
    #[error("{path}: {message}")]
    Io { path: String, message: String },
}

impl DataError {
    fn io(path: &Path, error: &std::io::Error) -> Self {
        Self::Io { path: path.display().to_string(), message: error.to_string() }
    }
}

/// The built-in generic layer. It has no numbers: public values are not shipped yet, so every
/// number rule that needs one reports "not covered" instead of inventing a limit.
pub fn generic_profile() -> Profile {
    Profile { format: 1, name: GENERIC_NAME.into(), version: GENERIC_VERSION.into(), layer: Layer::Generic, requirements: Vec::new() }
}

/// Parse and validate one whole file through the rule layer's types.
pub fn parse(bytes: &[u8]) -> Result<Profile, DataError> {
    let profile: Profile = serde_json::from_slice(bytes).map_err(|error| DataError::Invalid(format!("the data file is not a profile: {error}")))?;
    if profile.layer != Layer::Factory {
        return Err(DataError::Invalid("the data file must be a factory or standard profile".into()));
    }
    let probe = CheckInput {
        declared: Declared { process: Some("any".into()), material: Some("any".into()), thickness_mm: Some(1.0), thickness_from: None },
        generic: generic_profile(),
        shop: Some(profile.clone()),
        overrides: Vec::new(),
        facts: Default::default(),
        model: Default::default(),
        critical: Vec::new(),
    };
    solidcraft_dfm_rules::validate(&probe).map_err(|error| DataError::Invalid(error.to_string()))?;
    Ok(profile)
}

/// SHA-256 of the bytes, lower-case hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Read, validate and copy `bytes` into `root/.intcad/data/`. Nothing in the project is changed
/// before the whole file is valid; the copy has a unique name so an upgrade never overwrites the
/// copy the project still points at.
pub fn import_into(root: &Path, bytes: &[u8], kind: DataKind) -> Result<ImportedData, DataError> {
    if kind == DataKind::Generic {
        return Err(DataError::Invalid("generic data is built in and cannot be imported".into()));
    }
    let profile = parse(bytes)?;
    // The rule layer has one deciding external layer (`factory`); a published standard is a
    // factory-layer profile here, labelled by the project's `kind` only.
    if kind == DataKind::Generic {
        return Err(DataError::Invalid("generic data is built in and cannot be imported".into()));
    }
    let sha256 = sha256_hex(bytes);
    let short = sha256.get(..8).unwrap_or(&sha256);
    let file = format!(".intcad/data/{short}-{}.json", slug(&profile.name, &profile.version));
    let folder = root.join(FOLDER).join("data");
    fs::create_dir_all(&folder).map_err(|error| DataError::io(&folder, &error))?;
    let path = root.join(&file);
    let temporary = folder.join(format!(".{short}-{}.tmp", std::process::id()));
    fs::write(&temporary, bytes).map_err(|error| DataError::io(&temporary, &error))?;
    if let Err(error) = fs::rename(&temporary, &path) {
        let _ = fs::remove_file(&temporary);
        return Err(DataError::io(&path, &error));
    }
    Ok(ImportedData { kind, name: profile.name.clone(), version: profile.version.clone(), sha256, file, profile })
}

/// Why the selected shop does not apply to the declared values, if it does not.
pub fn applicability(shop: &Profile, declared: &Declared) -> Option<String> {
    let (Some(process), Some(material), Some(thickness)) = (declared.process.as_deref(), declared.material.as_deref(), declared.thickness_mm) else {
        return Some("the project has no declared process, material or thickness, so the shop's rows cannot be selected".into());
    };
    let (rows, gap) = solidcraft_dfm_rules::profile_data::applying(shop, process, material, thickness);
    if rows.is_empty() {
        let what = gap.map(|gap| gap.as_str()).unwrap_or("thickness");
        return Some(format!("the selected shop data has no row for this {what}"));
    }
    None
}

/// A file-name part that is stable and safe on every platform.
fn slug(name: &str, version: &str) -> String {
    let mut text = String::new();
    for character in format!("{name}-{version}").chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
            text.push(character.to_ascii_lowercase());
        } else {
            text.push('-');
        }
        if text.len() >= 40 {
            break;
        }
    }
    let trimmed = text.trim_matches(['-', '.']);
    if trimmed.is_empty() { "data".into() } else { trimmed.to_string() }
}

/// The path a project-relative data file names.
pub fn file_path(root: &Path, file: &str) -> PathBuf {
    root.join(file)
}

//! Host-owned prompt attachments for workflow turns.
//!
//! A composer image or PDF becomes an opaque `ArtifactRef`; only the reference crosses the
//! runtime protocol. The exact bytes stay here, scoped to the HostAi session and replaced at the
//! start of the next turn. This is deliberately separate from candidate/validation artifacts.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use super::{AiError, ArtifactRef, SessionKey};

/// Contract §4: four attachments per call, 4 MiB each, 16 MiB total.
pub(crate) const MAX_ATTACHMENTS: usize = 4;
pub(crate) const MAX_ATTACHMENT_BYTES: usize = 4 << 20;
pub(crate) const MAX_ATTACHMENT_TOTAL_BYTES: usize = 16 << 20;

pub(crate) const SUPPORTED_MEDIA_TYPES: &[&str] = &["image/png", "image/jpeg", "application/pdf"];

#[derive(Default)]
struct SessionAttachments {
    bytes: usize,
    entries: HashMap<String, std::sync::Arc<[u8]>>,
}

#[derive(Default)]
pub(crate) struct AttachmentStore {
    sessions: Mutex<HashMap<SessionKey, SessionAttachments>>,
}

impl AttachmentStore {
    fn sessions(&self) -> Result<MutexGuard<'_, HashMap<SessionKey, SessionAttachments>>, AiError> {
        self.sessions.lock().map_err(|_| AiError::internal("attachment_store"))
    }

    pub(crate) fn insert(&self, session: &SessionKey, artifact_id: &str, bytes: std::sync::Arc<[u8]>) -> Result<(), AiError> {
        if bytes.is_empty() || bytes.len() > MAX_ATTACHMENT_BYTES {
            return Err(AiError::exhausted("attachment_bytes", MAX_ATTACHMENT_BYTES as u32));
        }
        let mut sessions = self.sessions()?;
        let entry = sessions.entry(session.clone()).or_default();
        if entry.entries.len() >= MAX_ATTACHMENTS {
            return Err(AiError::exhausted("attachments", MAX_ATTACHMENTS as u32));
        }
        let total =
            entry.bytes.checked_add(bytes.len()).ok_or_else(|| AiError::exhausted("attachment_total_bytes", MAX_ATTACHMENT_TOTAL_BYTES as u32))?;
        if total > MAX_ATTACHMENT_TOTAL_BYTES {
            return Err(AiError::exhausted("attachment_total_bytes", MAX_ATTACHMENT_TOTAL_BYTES as u32));
        }
        entry.bytes = total;
        entry.entries.insert(artifact_id.to_string(), bytes);
        Ok(())
    }

    pub(crate) fn read(&self, session: &SessionKey, reference: &ArtifactRef) -> Option<std::sync::Arc<[u8]>> {
        let sessions = self.sessions.lock().ok()?;
        let entry = sessions.get(session)?.entries.get(&reference.artifact_id)?;
        (entry.len() == reference.size_bytes as usize).then(|| std::sync::Arc::clone(entry))
    }

    pub(crate) fn clear(&self, session: &SessionKey) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(session);
        }
    }
}

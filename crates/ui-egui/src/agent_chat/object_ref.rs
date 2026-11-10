//! Host object references for Anatomy answers.
//!
//! The host, not the model, decides which names are clickable. References come from the current
//! session's evaluated bodies and timeline features and carry the exact document id and revision
//! they were made from. A click resolves the reference against the live session before selecting;
//! a changed document, a changed revision or a deleted object is reported as unavailable and
//! never selects a same-name object.

use solidcraft_engine::{DocumentStamp, Sel, Session};

/// What a host reference points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    Body,
    Feature,
}

/// One clickable host object reference frozen to a document revision.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectRef {
    pub document_id: String,
    pub revision: u64,
    pub label: String,
    pub kind: RefKind,
    pub sel: Sel,
}

impl ObjectRef {
    pub fn new(document_id: impl Into<String>, revision: u64, label: impl Into<String>, kind: RefKind, sel: Sel) -> Self {
        Self { document_id: document_id.into(), revision, label: label.into(), kind, sel }
    }
}

/// Every body and feature the current document can resolve, longest name first so a name that
/// contains another is matched whole. Duplicate labels keep the first (body before feature).
pub fn collect(session: &mut Session) -> Vec<ObjectRef> {
    let stamp = session.document_stamp();
    let mut refs = Vec::new();
    for body in &session.model.state().bodies {
        let name = body.name.trim();
        if name.is_empty() {
            continue;
        }
        refs.push(ObjectRef::new(stamp.document_id.clone(), stamp.revision, name, RefKind::Body, Sel::Body { name: name.into() }));
    }
    for feature in &session.doc.features {
        let name = feature.name.trim();
        if name.is_empty() || feature.suppressed {
            continue;
        }
        refs.push(ObjectRef::new(stamp.document_id.clone(), stamp.revision, name, RefKind::Feature, Sel::Feature { id: feature.id }));
    }
    refs.sort_by(|a, b| b.label.len().cmp(&a.label.len()).then_with(|| a.label.cmp(&b.label)));
    refs.dedup_by(|a, b| a.label == b.label && a.kind == b.kind);
    refs
}

/// Resolve a reference against the live session. The document id and revision must still be the
/// ones the answer was made from, and the body or feature must still exist.
pub fn resolve(session: &mut Session, reference: &ObjectRef) -> Result<Sel, String> {
    let stamp: DocumentStamp = session.document_stamp();
    if reference.document_id != stamp.document_id {
        return Err(crate::i18n::tr("ui.agent_chat_object_other_document").to_string());
    }
    if reference.revision != stamp.revision {
        return Err(crate::i18n::tr("ui.agent_chat_object_stale").to_string());
    }
    match &reference.sel {
        Sel::Body { name } if session.model.state().body(name).is_some() => Ok(reference.sel.clone()),
        Sel::Feature { id } if session.doc.features.iter().any(|feature| feature.id == *id && !feature.suppressed) => Ok(reference.sel.clone()),
        _ => Err(crate::i18n::tr("ui.agent_chat_object_unavailable").to_string()),
    }
}

/// One piece of an Anatomy answer: plain text, or a host object name with its reference index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Segment {
    Text(String),
    Object { label: String, index: usize },
}

fn boundary_before(text: &str, at: usize) -> bool {
    text[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric() && c != '_')
}

fn boundary_after(text: &str, at: usize) -> bool {
    text[at..].chars().next().is_none_or(|c| !c.is_alphanumeric() && c != '_')
}

/// Split an answer into plain text and host object names. Only exact host labels at word
/// boundaries become clickable; the model cannot add a target by writing another name.
pub fn segments(text: &str, refs: &[ObjectRef]) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut plain_start = 0;
    let mut at = 0;
    while at < text.len() {
        let found = refs.iter().enumerate().find(|(_, reference)| {
            !reference.label.is_empty()
                && text[at..].starts_with(&reference.label)
                && boundary_before(text, at)
                && boundary_after(text, at + reference.label.len())
        });
        if let Some((index, reference)) = found {
            if plain_start < at {
                out.push(Segment::Text(text[plain_start..at].to_string()));
            }
            out.push(Segment::Object { label: reference.label.clone(), index });
            at += reference.label.len();
            plain_start = at;
        } else {
            at += text[at..].chars().next().map(char::len_utf8).unwrap_or(1);
        }
    }
    if plain_start < text.len() {
        out.push(Segment::Text(text[plain_start..].to_string()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session() -> Session {
        let mut session = Session::default();
        session.execute("solid.box", &json!({"length": 60, "width": 40, "height": 5, "body_name": "Cover"})).unwrap();
        session.execute("solid.box", &json!({"length": 60, "width": 40, "height": 20, "body_name": "Base"})).unwrap();
        session
    }

    #[test]
    fn references_are_host_owned_and_frozen_to_the_document_revision() {
        let mut session = session();
        let stamp = session.document_stamp();
        let refs = collect(&mut session);
        assert!(refs.iter().any(|reference| reference.label == "Cover" && reference.kind == RefKind::Body));
        assert!(refs.iter().any(|reference| reference.label == "Base" && reference.kind == RefKind::Body));
        assert!(refs.iter().all(|reference| reference.document_id == stamp.document_id && reference.revision == stamp.revision));
        assert!(refs.iter().filter(|reference| reference.label == "Cover").all(|reference| resolve(&mut session, reference).is_ok()));
    }

    #[test]
    fn a_name_that_contains_another_is_matched_whole() {
        let refs = vec![
            ObjectRef::new("doc", 1, "Cover plate", RefKind::Body, Sel::Body { name: "Cover plate".into() }),
            ObjectRef::new("doc", 1, "Cover", RefKind::Body, Sel::Body { name: "Cover".into() }),
        ];
        let segments = segments("The Cover plate sits on the Cover.", &refs);
        assert_eq!(
            segments,
            vec![
                Segment::Text("The ".into()),
                Segment::Object { label: "Cover plate".into(), index: 0 },
                Segment::Text(" sits on the ".into()),
                Segment::Object { label: "Cover".into(), index: 1 },
                Segment::Text(".".into()),
            ]
        );
    }

    #[test]
    fn a_stale_or_other_document_reference_is_unavailable() {
        let mut session = session();
        let refs = collect(&mut session);
        let cover = refs.iter().find(|reference| reference.label == "Cover").cloned().unwrap();
        assert!(resolve(&mut session, &cover).is_ok());
        let mut changed = cover.clone();
        changed.revision = changed.revision.saturating_add(1);
        assert!(resolve(&mut session, &changed).is_err());
        let mut other = cover;
        other.document_id = "another-document".into();
        assert!(resolve(&mut session, &other).is_err());
    }

    #[test]
    fn a_deleted_object_is_unavailable_and_never_falls_back_to_a_same_name() {
        let mut session = session();
        let refs = collect(&mut session);
        let cover = refs.iter().find(|reference| reference.label == "Cover").cloned().unwrap();
        session.execute("selection.delete", &json!({"items": [{"type": "body", "name": "Cover"}]})).unwrap();
        assert!(resolve(&mut session, &cover).is_err());
    }
}

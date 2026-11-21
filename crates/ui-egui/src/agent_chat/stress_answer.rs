//! After a turn in which a stress estimate solved, the chat reads the agent's answer for the
//! wording `how_to_report` asks for (PIL-294, decision 25, #201): no verdict on the part, and the
//! stress as a share of the yield strength. A miss adds a notice under the answer; the answer is
//! never changed. It runs where every backend's turn ends, and every backend's tools pass the
//! gate, so it covers every agent, not only the built-in one. The words are
//! [`solidcraft_engine::fea::report_words`], the list the engine's own wording is tested with.

use std::sync::Arc;

use serde_json::Value;
use solidcraft_engine::fea::{self, report_words};

use super::Item;
use crate::i18n::{format as trf, tr};

const COMMAND: &str = "inspect.stress_estimate";

/// A turn that asked to solve a stress estimate: its id, and the estimate shown before it asked.
#[derive(Default)]
pub(super) struct Watch {
    asked: Option<(u64, Option<Arc<fea::Field>>)>,
}

impl Watch {
    /// A request from turn `turn`'s agent, as it reaches the gate: noted when it would solve a
    /// stress estimate (on its own or in a batch, but not a dry run, which solves nothing).
    pub(super) fn request(&mut self, turn: u64, method: &str, params: &Value, document_id: &str) {
        if self.asked.as_ref().is_some_and(|(t, _)| *t == turn) || !solves(method, params) {
            return;
        }
        self.asked = Some((turn, fea::shown(document_id)));
    }

    /// Whether turn `turn` solved an estimate: it asked for one, and the estimate shown for the
    /// design is a new one since. Declined, refused, failed or stopped runs show nothing new.
    /// Forgets the turn either way.
    pub(super) fn solved(&mut self, turn: u64, document_id: &str) -> bool {
        let Some((asked, before)) = self.asked.take() else { return false };
        asked == turn && fea::shown(document_id).is_some_and(|now| !before.is_some_and(|b| Arc::ptr_eq(&b, &now)))
    }
}

/// Whether a request runs a stress estimate that solves, as the gate reads requests.
fn solves(method: &str, params: &Value) -> bool {
    let one = |id: Option<&Value>, p: Option<&Value>| {
        let estimate = id.and_then(Value::as_str).and_then(|id| solidcraft_engine::find_command(id.trim())).is_some_and(|c| c.id == COMMAND);
        estimate && p.and_then(|p| p.get("dry_run")).and_then(Value::as_bool) != Some(true)
    };
    match method {
        "engine.execute" => one(params.get("command"), params.get("params")),
        "engine.batch" => params
            .get("commands")
            .and_then(Value::as_array)
            .is_some_and(|steps| steps.iter().take(10_000).any(|s| one(s.get("command").or_else(|| s.get("id")), s.get("params")))),
        _ => false,
    }
}

/// What the agent showed the user since the last message: its answer, not its reasoning.
pub(super) fn answer(items: &[Item]) -> String {
    let from = items.iter().rposition(|i| matches!(i, Item::User { .. })).map_or(0, |i| i + 1);
    let parts: Vec<&str> = items
        .iter()
        .skip(from)
        .filter_map(|i| match i {
            Item::Agent { thought: false, text } | Item::AgentRefs { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    parts.join("\n")
}

/// The notice for an answer that judges the part or leaves out the share of yield; `None` when it
/// follows the rules.
pub(super) fn warning(answer: &str) -> Option<String> {
    let words = report_words::verdict_words(answer);
    let mut lines = Vec::new();
    if !words.is_empty() {
        lines.push(trf("ui.agent_chat_stress_verdict", &[("words", &words.join(", "))]));
    }
    if !report_words::gives_share_of_yield(answer) {
        lines.push(tr("ui.agent_chat_stress_no_share").to_string());
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// The estimate shown is one slot for the whole process ([`fea::show`]) and `cargo xtask test`
/// runs tests as threads of one: a test that shows one holds this until it is done.
#[cfg(test)]
pub(super) fn shown_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What a solved estimate leaves for design `document_id` (for tests: no mesher or solver runs).
#[cfg(test)]
pub(super) fn show_solved(document_id: &str) {
    fea::show(fea::Field {
        document_id: document_id.into(),
        body: "Body".into(),
        fingerprint: 0,
        positions: vec![[0.0; 3]; 3],
        normals: vec![[0.0, 0.0, 1.0]; 3],
        triangles: vec![[0, 1, 2]],
        stress: vec![106.0; 3],
        disp: vec![[0.0; 3]; 3],
        max_stress: 106.0,
        max_stress_at: [0.0; 3],
        max_disp: 0.8,
        max_disp_at: [0.0; 3],
        yield_mpa: 193.0,
        scale: fea::ScaleMode::Yield,
        quantity: fea::Quantity::Stress,
        hidden: false,
        deform_scale: 1.0,
        heading: "Estimate".into(),
        assumptions: Vec::new(),
        warnings: Vec::new(),
        loads: Vec::new(),
        supports: Vec::new(),
        away_stress: None,
        equilibrium: None,
        mesh_check: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_solve_is_an_estimate_run_alone_or_in_a_batch_and_never_a_dry_run() {
        let estimate = |p: Value| json!({"command": "inspect.stress_estimate", "params": p});
        assert!(solves("engine.execute", &estimate(json!({"loads": []}))));
        assert!(solves("engine.execute", &json!({"command": " Inspect.Stress_Estimate ", "params": {}})), "as the engine finds commands");
        assert!(solves("engine.execute", &json!({"command": "inspect.stress_estimate"})), "no params");
        assert!(solves("engine.batch", &json!({"commands": [{"command": "model.faces"}, {"id": "inspect.stress_estimate", "params": {}}]})));
        assert!(!solves("engine.execute", &estimate(json!({"dry_run": true}))));
        assert!(!solves("engine.batch", &json!({"commands": [estimate(json!({"dry_run": true}))]})));
        assert!(!solves("engine.execute", &json!({"command": "inspect.stress_estimate.show", "params": {"show": "deflection"}})));
        assert!(!solves("document.inspect", &estimate(json!({}))));
        for junk in [json!(null), json!([]), json!({"command": 7}), json!({"commands": "x"}), json!({"commands": [null, 3]})] {
            assert!(!solves("engine.execute", &junk) && !solves("engine.batch", &junk), "{junk}");
        }
    }

    #[test]
    fn a_turn_solved_when_a_new_estimate_is_shown_after_it_asked() {
        let _shown = shown_lock();
        let doc = format!("stress-answer-watch-{}", std::process::id());
        let ask = json!({"command": "inspect.stress_estimate", "params": {}});
        let mut w = Watch::default();
        // Asked, and a new estimate shown: solved, once.
        w.request(1, "engine.execute", &ask, &doc);
        show_solved(&doc);
        assert!(w.solved(1, &doc));
        assert!(!w.solved(1, &doc), "forgotten once read");
        // Asked, nothing new shown (declined, refused, failed): not solved.
        w.request(2, "engine.execute", &ask, &doc);
        assert!(!w.solved(2, &doc));
        // A new estimate shown without being asked for (Show, from the Inspect tab): not this turn's.
        show_solved(&doc);
        assert!(!w.solved(3, &doc));
        // Asked by another turn: not this one's.
        w.request(4, "engine.execute", &ask, &doc);
        show_solved(&doc);
        assert!(!w.solved(5, &doc));
        // A second request in the same turn keeps what was shown before the first.
        w.request(6, "engine.execute", &ask, &doc);
        show_solved(&doc);
        w.request(6, "engine.execute", &ask, &doc);
        assert!(w.solved(6, &doc));
    }

    #[test]
    fn the_answer_is_what_the_agent_showed_since_the_last_message() {
        let user = |text: &str| Item::User { text: text.into(), image: false, pictures: Vec::new(), context: Default::default(), files: Vec::new() };
        let agent = |thought: bool, text: &str| Item::Agent { thought, text: text.into() };
        let items = [
            user("first"),
            agent(false, "The part is safe."),
            user("How much stress?"),
            agent(true, "Is it safe? I must not say."),
            agent(false, "About 55 % of the yield strength."),
            Item::Notice { text: "safe".into(), error: false },
            Item::AgentRefs { text: "Peak near the root.".into(), refs: Vec::new() },
        ];
        assert_eq!(answer(&items), "About 55 % of the yield strength.\nPeak near the root.");
        assert_eq!(answer(&[]), "");
    }
}

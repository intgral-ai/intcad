//! The stress estimate's release switch (spec decision 32): what it hides from people at each
//! level, what it refuses agents, and that a person's own request is never refused.

use serde_json::{Value, json};
use solidcraft_engine::entitlement::EntryCategory;
use std::sync::mpsc::Receiver;

use super::*;
use crate::control::{ControlRequest, ControlResponse};

const CMD: &str = "inspect.stress_estimate";
const SHOW: &str = "inspect.stress_estimate.show";

fn app_at(level: &str) -> SolveApp {
    let mut app = SolveApp::new(solidcraft_engine::Session::default(), crate::Services::default());
    app.preferences.stress_estimate = level.into();
    app
}

/// A 10 × 20 × 30 mm block, with the copy path on as the desktop sets it.
fn block_at(level: &str) -> SolveApp {
    let mut app = app_at(level);
    app.run("solid.box", json!({"length": 10, "width": 20, "height": 30})).unwrap();
    app.apply_off_thread = Some(std::time::Duration::from_secs(1));
    app
}

/// The index of `Body1`'s face with a triangle wholly at height `z`.
fn face_at_height(app: &SolveApp, z: f64) -> usize {
    let st = crate::open_meshing::session_world(&app.session);
    let m = st.body("Body1").unwrap().mesh();
    let (_, f) = m.triangles.iter().zip(&m.tri_face).find(|(t, _)| m.tri(t).is_some_and(|ps| ps.iter().all(|p| (p.z - z).abs() < 1e-9))).unwrap();
    *f as usize
}

/// The block held at its bottom and pushed down on its top with 500 N, in 6061-T6.
fn estimate(app: &SolveApp, dry_run: bool) -> Value {
    let (bottom, top) = (face_at_height(app, 0.0), face_at_height(app, 30.0));
    let mut params = json!({
        "body": "Body1",
        "supports": [{"face": [bottom], "type": "fixed"}],
        "loads": [{"face": [top], "force": [0.0, 0.0, -500.0]}],
        "material": "6061-T6",
    });
    if dry_run {
        params["dry_run"] = json!(true);
    }
    params
}

/// Drive the app's queue until `reply` answers (the copy's worker, then its settlement).
fn answer(app: &mut SolveApp, reply: &Receiver<ControlResponse>) -> Value {
    let ctx = crate::fonts::test_context();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        app.drain_control(&ctx);
        app.poll_control_jobs(&ctx);
        if let Ok(v) = reply.try_recv() {
            return v;
        }
        assert!(std::time::Instant::now() < until, "no answer in 60 s");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn refused_by_the_switch(v: &Value) -> bool {
    v["ok"] == false && v["error"].as_str().is_some_and(|e| e.contains("not available to agents"))
}

/// S6: `auto` is a development build's level here (a test is never a release build), the
/// preference's names are the engine's, and the app's release test is the engine's.
#[test]
fn auto_is_this_builds_level_and_a_set_value_wins() {
    assert_eq!(crate::update::release_build(), solidcraft_engine::fea::offer::release_build(), "one test for a release build");
    assert_eq!(SolveApp::new(solidcraft_engine::Session::default(), crate::Services::default()).preferences.stress_estimate, "auto");
    assert_eq!(offer(&app_at("auto")), Offer::this_build());
    assert_eq!(offer(&app_at("auto")), Offer::Agents, "a test is a development build");
    assert_eq!(offer(&app_at("off")), Offer::Off);
    assert_eq!(offer(&app_at("by_hand")), Offer::ByHand);
    assert_eq!(offer(&app_at("agents")), Offer::Agents);
    assert_eq!(offer(&app_at("yes please")), Offer::Off, "an unknown value never turns it on");
    // Saved and read back, `auto` stays `auto`: a later release's default reaches it.
    let saved = serde_json::to_value(&app_at("auto").preferences).unwrap();
    assert_eq!(saved[PREFERENCE], "auto");
}

/// S1 and S2: below `agents` the Stress button is nowhere a person looks (the ribbon's
/// Inspect ▸ Estimate as drawn, command search) and starting it opens nothing; at `agents` it is
/// there and opens the chat, as before the switch. #270 lowers the button to `by_hand`.
#[test]
fn the_stress_button_is_hidden_until_the_switch_offers_it() {
    for level in ["off", "by_hand", "agents"] {
        let shown = level == "agents";
        let mut h = crate::scenario::Harness::new();
        h.app.home.open = false;
        h.app.preferences.stress_estimate = level.into();
        h.call("ui.resize", json!({"width": 1600, "height": 900}));
        h.app.run("solid.box", json!({"length": 10, "width": 20, "height": 30})).unwrap();
        h.app.ui.tab = "INSPECT".into();
        h.frames(2);
        assert_eq!(crate::scenario::rect_of("panel:INSPECT:ESTIMATE").is_some(), shown, "{level}: the ribbon's Estimate panel");
        assert!(crate::scenario::rect_of("panel:INSPECT:MEASURE").is_some(), "{level}: the rest of the tab is drawn");
        let lists = crate::toolbar_custom::shown_panel_lists(&h.app, "INSPECT");
        assert_eq!(lists.iter().any(|(_, cmds, _)| cmds.iter().any(|c| c.id == CMD)), shown, "{level}: its drop-down");
        let found: Vec<&str> = crate::shortcut_box::search(&h.app, "stress").into_iter().map(|row| row.0).collect();
        assert_eq!(found.contains(&CMD), shown, "{level}: command search {found:?}");
        assert!(!found.contains(&SHOW) || shown, "{level}: {found:?}");
        h.app.status = None;
        h.app.start_at(CMD, None);
        assert_eq!(h.app.agent_chat.open, shown, "{level}: the button opens the chat only at agents");
        let status = h.app.status.as_ref().map(|s| s.0.clone()).unwrap_or_default();
        let not_offered = h.app.tr("ui.stress_estimate_not_offered").to_string();
        assert!(!not_offered.starts_with("ui."), "the words are in the catalog");
        assert_eq!(status.contains(&not_offered), !shown, "{level}: {status:?}");
        // `ui.start`, as a shortcut or the harness starts a command, reaches the same place.
        h.app.agent_chat.open = false;
        h.app.status = None;
        h.call("ui.start", json!({"command": CMD}));
        assert_eq!(h.app.agent_chat.open, shown, "{level}: ui.start");
        let status = h.app.status.as_ref().map(|s| s.0.clone()).unwrap_or_default();
        assert_eq!(status.contains(&not_offered), !shown, "{level}: ui.start says why: {status:?}");
        // "Did you mean" never names it while it is hidden.
        assert_eq!(crate::shortcut_box::closest("stress estimat", crate::stress_switch::button(&h.app)) == Some("Stress Estimate"), shown, "{level}");
    }
}

/// S3: below `agents`, an agent's estimate is refused before it starts by every entry an agent
/// has (the chat's bridge, the packaged Agent, ACP and MCP agents, the control port), as a solve,
/// a dry run, inside a batch or a script, showing a result, or started as a command: nothing
/// reaches the copy path, and the agent is told not to ask again. At `agents` the switch lets it
/// by (today the copy path then refuses it, #240; #269 charges it).
#[test]
fn an_agents_estimate_is_refused_before_it_starts_below_agents() {
    type Make = fn(String, Value) -> (ControlRequest, Receiver<ControlResponse>);
    let entries: [(EntryCategory, Make); 5] = [
        (EntryCategory::GuiBridgeMcp, |m, p| ControlRequest::trusted_bridge(m, p)),
        (EntryCategory::Mastra, |m, p| ControlRequest::trusted_mastra(m, p)),
        (EntryCategory::ExternalAcpMcp, |m, p| ControlRequest::trusted_acp(m, p)),
        (EntryCategory::HeadlessMcp, |m, p| ControlRequest::trusted_mcp(m, p)),
        (EntryCategory::DirectControl, |m, p| ControlRequest::trusted_control(m, p)),
    ];
    for level in ["off", "by_hand", "agents"] {
        let mut app = block_at(level);
        // At `agents` a solve would be the copy path's to refuse or run: dry runs only there.
        let solve = if level == "agents" { estimate(&app, true) } else { estimate(&app, false) };
        let requests = [
            ("engine.execute", json!({"command": CMD, "params": estimate(&app, true)})),
            ("engine.execute", json!({"command": CMD, "params": solve})),
            ("engine.execute", json!({"command": " Inspect.Stress_Estimate ", "params": estimate(&app, true)})),
            ("engine.execute", json!({"command": SHOW, "params": {"show": "stress"}})),
            ("engine.batch", json!({"commands": [{"command": "inspect.measure", "params": {}}, {"command": CMD, "params": estimate(&app, true)}]})),
            ("engine.script", json!([{"command": CMD, "params": estimate(&app, true)}])),
            ("ui.start", json!({"command": CMD})),
        ];
        for (entry, make) in entries {
            for (method, params) in &requests {
                let (req, reply) = make(method.to_string(), params.clone());
                assert_eq!(req.entry(), entry);
                app.control_held.push_back(req);
                let v = answer(&mut app, &reply);
                let what = format!("{level}, {entry:?}, {method} {params}");
                if level == "agents" {
                    assert!(!refused_by_the_switch(&v), "{what}: the switch lets it by: {v}");
                } else {
                    assert!(refused_by_the_switch(&v), "{what}: {v}");
                    assert!(v["error"].as_str().unwrap().contains("do not ask for it again"), "{what}: {v}");
                    assert!(app.control_job.is_none() && app.control_answering.is_empty(), "{what}: nothing on the copy path");
                    assert!(!app.agent_chat.open, "{what}: ui.start opened nothing");
                }
            }
        }
        // The same refusal for a request handed straight to the handler, past the queue.
        let (mut req, _reply) = ControlRequest::trusted_bridge("engine.execute", json!({"command": CMD, "params": estimate(&app, true)}));
        let out = match crate::control::handle(&mut app, &crate::fonts::test_context(), &mut req) {
            crate::control::Outcome::Done(v) => v,
            _ => Value::Null,
        };
        assert_eq!(refused_by_the_switch(&out), level != "agents", "{level}: control::handle: {out}");
    }
}

/// S4: a person's own estimate (the free GUI entry #270's Run makes) is never refused by the
/// switch, at any level: its dry run on the block is answered on the copy path.
#[test]
fn a_persons_own_estimate_is_never_refused_by_the_switch() {
    for level in ["off", "by_hand", "agents", "auto"] {
        let mut app = block_at(level);
        let (req, reply) = ControlRequest::trusted_gui("engine.execute", json!({"command": CMD, "params": estimate(&app, true)}));
        assert!(crate::control::free_entry(req.entry()));
        app.control_held.push_back(req);
        let v = answer(&mut app, &reply);
        assert_eq!(v["ok"], true, "{level}: {v}");
        assert!(v["result"]["setup"].as_array().is_some_and(|s| !s.is_empty()), "{level}: {v}");
    }
}

/// Below `agents`, the command list an agent reads (`engine.commands`, whoever asks) leaves the
/// estimate out; at `agents` it is listed, as before the switch. Asked on the free entry: a test
/// app has no account, and a bridge request is refused there before it runs anything.
#[test]
fn the_agents_command_list_leaves_the_estimate_out_below_agents() {
    for level in ["off", "by_hand", "agents"] {
        let mut app = app_at(level);
        let (req, reply) = ControlRequest::trusted_gui("engine.commands", json!({}));
        app.control_held.push_back(req);
        let v = answer(&mut app, &reply);
        let ids: Vec<&str> = v["result"].as_array().unwrap_or_else(|| panic!("{level}: {v}")).iter().filter_map(|c| c["id"].as_str()).collect();
        assert!(ids.contains(&"inspect.measure"), "{level}: the rest is listed");
        assert_eq!(ids.contains(&CMD), level == "agents", "{level}");
        assert_eq!(ids.contains(&SHOW), level == "agents", "{level}");
    }
}

/// An agent on the control port cannot raise the switch: `ui.prefs` from a controlled entry may
/// not set it, whatever else it may do there. A person's own request may.
#[test]
fn an_agent_cannot_turn_its_own_route_on() {
    let raise = json!({PREFERENCE: "agents"});
    for entry in EntryCategory::CONTROLLED {
        let why = set_by_control_refusal(entry, &raise).unwrap_or_default();
        assert!(why.contains("set by a person") && why.contains(solidcraft_engine::fea::offer::ENV), "{entry:?}: {why}");
        assert_eq!(set_by_control_refusal(entry, &json!({"right_rail": false})), None, "{entry:?}: other preferences as before");
    }
    for entry in [EntryCategory::HumanGuiFree, EntryCategory::WebGuiFree] {
        assert_eq!(set_by_control_refusal(entry, &raise), None, "{entry:?}");
    }
    let mut app = app_at("off");
    let ctx = crate::fonts::test_context();
    for (mut req, _reply) in [
        ControlRequest::trusted_control("ui.prefs", json!({PREFERENCE: "agents"})),
        ControlRequest::trusted_bridge("ui.prefs", json!({PREFERENCE: "agents"})),
    ] {
        let out = match crate::control::handle(&mut app, &ctx, &mut req) {
            crate::control::Outcome::Done(v) => v,
            _ => Value::Null,
        };
        assert_ne!(out["ok"], true, "{out}");
        assert_eq!(offer(&app), Offer::Off, "{out}");
    }
    let (mut req, _reply) = ControlRequest::trusted_gui("ui.prefs", json!({PREFERENCE: "by_hand"}));
    let out = match crate::control::handle(&mut app, &ctx, &mut req) {
        crate::control::Outcome::Done(v) => v,
        _ => Value::Null,
    };
    assert_eq!(out["ok"], true, "{out}");
    assert_eq!(offer(&app), Offer::ByHand);
}

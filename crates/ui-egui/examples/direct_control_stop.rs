//! Measure control dispatch without creating a window, listener, or billing authority.

#[path = "../../engine/examples/controlled_stop_support/mod.rs"]
mod controlled_stop_support;

use controlled_stop_support::{CUTS, Result, elapsed, header, plate, sample};
use serde_json::{Value, json};
use solidcraft_ui_egui::control::{self, ControlRequest, Outcome};
use solidcraft_ui_egui::{Services, SolveApp};

fn main() -> Result<()> {
    header("cut features / phase");
    let ctx = egui::Context::default();
    let params = json!({"command":"inspect.measure","params":{"names_only":true}});
    let wire = serde_json::to_vec(&params)?;
    for cuts in CUTS {
        let session = plate(cuts)?;
        let mut app = SolveApp::new(session, Services::default());
        sample(format!("{cuts} / direct entry (refused)"), || {
            elapsed(|| {
                let (mut req, _) = ControlRequest::trusted_control("engine.execute", serde_json::from_slice::<Value>(&wire)?);
                match control::handle(&mut app, &ctx, &mut req) {
                    Outcome::Done(reply) if reply["ok"] == false => Ok(()),
                    _ => Err("an unauthorized direct request was not refused".into()),
                }
            })
        })?;
        sample(format!("{cuts} / shared control dispatch"), || {
            elapsed(|| {
                let (mut req, rx) = ControlRequest::trusted_gui("engine.execute", serde_json::from_slice::<Value>(&wire)?);
                let Outcome::Done(reply) = control::handle(&mut app, &ctx, &mut req) else {
                    return Err("expected synchronous control dispatch".into());
                };
                req.reply.send(reply)?;
                let result = serde_json::to_vec(&rx.recv()?)?;
                let reply: Value = serde_json::from_slice(&result)?;
                if reply["ok"] != true || reply["result"]["body_count"] != 1 {
                    return Err(format!("unexpected control response: {reply}").into());
                }
                Ok(())
            })
        })?;
        sample(format!("{cuts} / direct engine only"), || {
            elapsed(|| {
                if app.session.execute("inspect.measure", &json!({"names_only":true}))?["body_count"] != 1 {
                    return Err("engine reply missing body".into());
                }
                Ok(())
            })
        })?;
    }
    Ok(())
}

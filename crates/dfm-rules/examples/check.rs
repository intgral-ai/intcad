//! Run the pure rule layer on one JSON `CheckInput`.
//!
//! ```sh
//! cargo run -p solidcraft-dfm-rules --example check -- input.json
//! ```
//!
//! This is a hand entry for a recorded or hand-written input. The App uses
//! `solidcraft_engine::dfm::check_sheet`, which measures the facts first. The library itself never
//! reads a file.

use std::process::ExitCode;

use solidcraft_dfm_rules::{CheckInput, canonical_json, check};

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: check INPUT.json");
        return ExitCode::from(2);
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("read {path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let input: CheckInput = match serde_json::from_str(&text) {
        Ok(input) => input,
        Err(error) => {
            eprintln!("parse {path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    match check(&input) {
        Ok(report) => match canonical_json(&report) {
            Ok(text) => {
                println!("{text}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

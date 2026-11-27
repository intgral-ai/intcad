//! Desktop startup from the command line. `--help`, `--version` and `--crash-log` answer and
//! return before the launch opens its settings folder or the account ledger. The fixture is
//! isolated and inert: no display, credential store or network is needed, and nothing is read
//! from the user's own folders.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("intcad-information-only-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn run(root: &Path, argument: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_intcad"))
        .arg(argument)
        .env("HOME", root.join("home"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("DBUS_SESSION_BUS_ADDRESS", root.join("no-session-bus"))
        .env("SOLIDCRAFT_SETTINGS_DIR", root.join("isolated-settings"))
        .env("SOLIDCRAFT_CRASH_DIR", root.join("crashes"))
        .output()
        .unwrap()
}

#[test]
fn information_only_arguments_leave_an_absent_settings_folder_absent() {
    let root = fixture("flags");
    for (argument, answer) in [("--help", "usage:"), ("--version", "intcad "), ("--crash-log", "crash reports:")] {
        let output = run(&root, argument);
        assert!(output.status.success(), "{argument} failed: {output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains(answer), "{argument} answered: {output:?}");
        let settings = root.join("isolated-settings");
        assert!(!settings.exists(), "{argument} created {}", settings.display());
    }
    let _ = std::fs::remove_dir_all(&root);
}

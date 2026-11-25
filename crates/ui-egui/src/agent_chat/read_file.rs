//! A file the agent read, shown as a card under its tool row. Which rows have one, and what a
//! click on the card does: a text file is previewed in the panel, a document (a PDF, a Word file)
//! is handed to the program the system has for it. Only a file inside the agent's own working
//! folder gets a card, so nothing the agent merely names is opened or shown.

use serde_json::Value;
use std::path::{Path, PathBuf};

/// What a click on a file's card does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    /// Its text is shown in the panel; `markdown`: drawn as Markdown, not as it is written.
    Preview { markdown: bool },
    /// The system's program for it opens it.
    Open,
}

/// The most of a file that is shown in the panel. The page gets it with the chat's state, which
/// is sent again whenever the chat changes, so it is kept small.
pub const MAX_PREVIEW: usize = 64 * 1024;

/// What a click does for a file of this name, by its extension; `None`: the file gets no card.
/// Text that a program would run or draw (HTML) is only ever shown as text. Only documents are
/// handed to the system: nothing that the system would run.
pub fn act_of(name: &Path) -> Option<Act> {
    let extension = name.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "md" | "markdown" => Some(Act::Preview { markdown: true }),
        "txt" | "csv" | "tsv" | "json" | "jsonl" | "yaml" | "yml" | "toml" | "xml" | "html" | "htm" | "log" => Some(Act::Preview { markdown: false }),
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "rtf" | "odt" | "ods" | "odp" => Some(Act::Open),
        _ => None,
    }
}

/// The path a finished read names: `{"path": …}` in its arguments, as Mastra Code's `view` has it.
pub fn named<'a>(kind: &str, completed: bool, input: Option<&'a Value>) -> Option<&'a str> {
    let path = input?.get("path")?.as_str()?;
    (kind == "read" && completed && !path.is_empty() && path.len() <= 4096 && !path.chars().any(char::is_control)).then_some(path)
}

/// The file `path` names, when it is a file inside the agent's working folder `work` that gets a
/// card: its real place (links followed) and what a click does. The folder, the kind of file and
/// the extension are all those of that real place, so a link named `notes.pdf` that leads to a
/// script, or out of the folder, gives nothing.
#[cfg(not(target_arch = "wasm32"))]
pub fn resolve(work: &Path, path: &str) -> Option<(PathBuf, Act)> {
    let work = std::fs::canonicalize(work).ok()?;
    let real = std::fs::canonicalize(work.join(path)).ok()?;
    (real.starts_with(&work) && real.is_file()).then(|| act_of(&real).map(|act| (real, act))).flatten()
}

#[cfg(target_arch = "wasm32")]
pub fn resolve(_work: &Path, _path: &str) -> Option<(PathBuf, Act)> {
    None
}

/// The start of a file as text, for the panel: at most [`MAX_PREVIEW`] bytes of it, ending in `…`
/// when there is more. Bytes that are not UTF-8 show as the replacement character.
#[cfg(not(target_arch = "wasm32"))]
pub fn preview(file: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(file)?.take(MAX_PREVIEW as u64 + 1).read_to_end(&mut bytes)?;
    let more = bytes.len() > MAX_PREVIEW;
    bytes.truncate(MAX_PREVIEW);
    // A cut in the middle of a character leaves a broken one at the end: drop it.
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if more {
        if text.ends_with('\u{FFFD}') {
            text.pop();
        }
        text.push('…');
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn what_a_click_does_goes_by_the_extension() {
        let act = |name: &str| act_of(Path::new(name));
        assert_eq!(act("notes.md"), Some(Act::Preview { markdown: true }));
        assert_eq!(act("README.MARKDOWN"), Some(Act::Preview { markdown: true }));
        for text in ["a.txt", "a.csv", "a.json", "a.yaml", "page.html", "run.log"] {
            assert_eq!(act(text), Some(Act::Preview { markdown: false }), "{text}");
        }
        for document in ["spec.pdf", "spec.PDF", "a.docx", "a.doc", "a.xlsx", "a.pptx", "a.rtf", "a.odt"] {
            assert_eq!(act(document), Some(Act::Open), "{document}");
        }
        // Nothing the system would run, no picture (it has its own card), nothing without a type.
        for none in ["run.sh", "a.command", "a.app", "a.exe", "a.bat", "a.py", "a.js", "a.png", "Makefile", ".pdf", "a."] {
            assert_eq!(act(none), None, "{none}");
        }
    }

    /// The rows as the built-in agent sent them when the owner tried each kind of file.
    #[test]
    fn a_finished_read_names_its_file() {
        let input = json!({"path": "display-tests/test.pdf"});
        assert_eq!(named("read", true, Some(&input)), Some("display-tests/test.pdf"));
        assert_eq!(named("read", false, Some(&input)), None, "a read still running");
        assert_eq!(named("execute", true, Some(&input)), None, "a command that has a path");
        assert_eq!(named("read", true, None), None);
        for other in
            [json!({"path": ""}), json!({"path": 7}), json!({"file": "a.pdf"}), json!({"path": "a\nb.pdf"}), json!({"path": "a".repeat(5000)})]
        {
            assert_eq!(named("read", true, Some(&other)), None, "{other}");
        }
    }

    #[test]
    fn only_a_file_inside_the_agents_folder_gets_a_card() {
        let root = std::env::temp_dir().join(format!("intcad-read-file-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (work, outside) = (root.join("work"), root.join("outside"));
        std::fs::create_dir_all(work.join("display-tests")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        for file in ["display-tests/test.pdf", "display-tests/test.md", "display-tests/test.docx", "display-tests/test.png", "display-tests/run.sh"] {
            std::fs::write(work.join(file), b"x").unwrap();
        }
        std::fs::write(outside.join("secret.md"), b"x").unwrap();
        let act = |path: &str| resolve(&work, path).map(|(_, act)| act);

        assert_eq!(act("display-tests/test.pdf"), Some(Act::Open));
        assert_eq!(act("display-tests/test.md"), Some(Act::Preview { markdown: true }));
        // The row whose result was "binary file not readable as text": the file is still there to open.
        assert_eq!(act("display-tests/test.docx"), Some(Act::Open));
        let real = std::fs::canonicalize(work.join("display-tests/test.md")).unwrap();
        assert_eq!(resolve(&work, real.to_str().unwrap()).map(|(file, _)| file), Some(real), "an absolute path inside the folder");
        // No card: a picture, a script, a folder, a file that is not there, a file outside.
        for none in ["display-tests/test.png", "display-tests/run.sh", "display-tests", "display-tests/gone.pdf", "../outside/secret.md"] {
            assert_eq!(act(none), None, "{none}");
        }
        assert_eq!(act(outside.join("secret.md").to_str().unwrap()), None, "an absolute path outside the folder");
        #[cfg(unix)]
        {
            // A link is read as what it leads to: out of the folder, or to a script under a document's name.
            std::os::unix::fs::symlink(outside.join("secret.md"), work.join("link.md")).unwrap();
            std::os::unix::fs::symlink(work.join("display-tests/run.sh"), work.join("notes.pdf")).unwrap();
            std::os::unix::fs::symlink(work.join("display-tests/test.pdf"), work.join("same.pdf")).unwrap();
            assert_eq!(act("link.md"), None);
            assert_eq!(act("notes.pdf"), None);
            assert_eq!(act("same.pdf"), Some(Act::Open));
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_preview_is_the_start_of_the_file() {
        let root = std::env::temp_dir().join(format!("intcad-read-preview-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let (short, long, binary) = (root.join("short.md"), root.join("long.txt"), root.join("odd.txt"));
        std::fs::write(&short, "# 标题\n").unwrap();
        // Three bytes a character: the cut falls inside one, which is dropped.
        std::fs::write(&long, "标".repeat(MAX_PREVIEW)).unwrap();
        std::fs::write(&binary, [b'a', 0xFF, b'b']).unwrap();
        assert_eq!(preview(&short).unwrap(), "# 标题\n");
        let cut = preview(&long).unwrap();
        assert!(cut.len() <= MAX_PREVIEW + '…'.len_utf8() && cut.ends_with("标…") && !cut.contains('\u{FFFD}'), "{} bytes", cut.len());
        assert_eq!(preview(&binary).unwrap(), "a\u{FFFD}b");
        assert!(preview(&root.join("gone.txt")).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}

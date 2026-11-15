//! The `outline` tool (`docs/architecture/05-tools.md`, "`outline`"), with
//! an outline source of the test's own: the tools cannot see the index.

use std::sync::{Arc, Mutex};

use cortex_base::AbsPath;
use cortex_tools::{Effect, OutlineTool, Outlined, Outliner, OutputLimits, ReadGate, Tool};
use serde_json::json;

use super::{Workspace, failed, ok};

/// Outlines each file by its own lines that start with `fn `, and
/// remembers what it was asked.
#[derive(Default)]
struct FakeOutliner {
    asked: Mutex<Vec<String>>,
}

impl Outliner for FakeOutliner {
    fn outline(&self, path: &AbsPath) -> Option<String> {
        self.describe(path).ok().map(|outlined| outlined.text)
    }

    fn describe(&self, path: &AbsPath) -> Result<Outlined, String> {
        self.asked
            .lock()
            .unwrap()
            .push(path.file_name().unwrap_or("").to_owned());
        let bytes = std::fs::read(path.to_fs_path()).map_err(|error| error.to_string())?;
        if bytes.contains(&0) {
            return Err("it is a binary file".to_owned());
        }
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<String> = text
            .lines()
            .enumerate()
            .filter(|(_, line)| line.starts_with("fn "))
            .map(|(at, line)| format!("{}  {line}", at + 1))
            .collect();
        let language = path
            .file_name()
            .is_some_and(|name| name.ends_with(".rs"))
            .then(|| "rust".to_owned());
        let size = bytes.len() as u64;
        let outlined = Outlined::new(lines.join("\n"), language, lines.len()).with_size(size);
        Ok(if size > 1000 {
            outlined.covering(1000)
        } else {
            outlined
        })
    }
}

fn with_outliner(ws: &Workspace) -> (cortex_tools::RunCx, Arc<FakeOutliner>) {
    let outliner = Arc::new(FakeOutliner::default());
    let cx = ws
        .cx()
        .with_outliner(Arc::clone(&outliner) as Arc<dyn Outliner>);
    (cx, outliner)
}

#[test]
fn a_files_outline_has_its_lines_and_says_what_it_is() {
    let ws = Workspace::new("outline_file");
    ws.file("src/lib.rs", "// a\nfn one() {}\n\nfn two() {}\n");
    let (cx, _) = with_outliner(&ws);
    let text = ok(&ws.call_with(&cx, &OutlineTool::new(), json!({"path": "src/lib.rs"})));
    assert_eq!(
        text,
        "src/lib.rs (rust, 2 definitions; approximate):\n2  fn one() {}\n4  fn two() {}"
    );
}

#[test]
fn a_file_with_no_extractor_or_over_the_cap_says_so() {
    let ws = Workspace::new("outline_fallback");
    ws.file("notes.txt", "fn looks() {}\n");
    ws.file("big.rs", &"fn f() {}\n".repeat(200));
    let (cx, _) = with_outliner(&ws);
    let tool = OutlineTool::new();
    let text = ok(&ws.call_with(&cx, &tool, json!({"path": "notes.txt"})));
    assert!(text.starts_with("notes.txt (1 definition, from its indentation; approximate):"));
    assert!(
        text.contains("No extractor reads this kind of file"),
        "{text}"
    );
    let text = ok(&ws.call_with(&cx, &tool, json!({"path": "big.rs"})));
    assert!(
        text.ends_with("this outline covers its first 1000 bytes.]"),
        "{text}"
    );
}

#[test]
fn a_binary_file_has_no_outline_and_says_why() {
    let ws = Workspace::new("outline_binary");
    ws.bytes("blob.bin", b"\x7fELF\0\0");
    let (cx, _) = with_outliner(&ws);
    let text = failed(&ws.call_with(&cx, &OutlineTool::new(), json!({"path": "blob.bin"})));
    assert_eq!(text, "`blob.bin` has no outline: it is a binary file.");
}

#[test]
fn a_path_outside_the_workspace_is_refused_and_nothing_is_read() {
    let ws = Workspace::new("outline_outside");
    let outside = ws.dir.join("outside.rs");
    std::fs::write(&outside, "fn secret() {}\n").unwrap();
    let (cx, outliner) = with_outliner(&ws);
    let tool = OutlineTool::new();
    for path in [
        outside.to_string_lossy().into_owned(),
        "../outside.rs".to_owned(),
    ] {
        let result = ws.call_with(&cx, &tool, json!({"path": path}));
        let text = failed(&result);
        assert!(
            text.contains("outside the workspace") || text.contains("leads out of"),
            "{text}"
        );
    }
    assert!(
        outliner.asked.lock().unwrap().is_empty(),
        "nothing was outlined"
    );
}

/// A gate that keeps anything named `secret.rs` out of a walk.
struct NoSecret;

impl ReadGate for NoSecret {
    fn may_read(&self, path: &AbsPath) -> bool {
        path.file_name() != Some("secret.rs")
    }
}

#[test]
fn a_directorys_outline_is_its_files_in_order_down_to_its_depth_past_the_gate() {
    let ws = Workspace::new("outline_dir");
    ws.file("src/b.rs", "fn b() {}\n");
    ws.file("src/a.rs", "fn a() {}\n");
    ws.file("src/secret.rs", "fn hidden() {}\n");
    ws.file("src/empty.rs", "// nothing\n");
    ws.file("src/deep/c.rs", "fn c() {}\n");
    ws.file("src/.gitignore", "ignored.rs\n");
    ws.file("src/ignored.rs", "fn ignored() {}\n");
    let (cx, outliner) = with_outliner(&ws);
    let cx = cx.with_gate(Arc::new(NoSecret));
    let tool = OutlineTool::new();
    let text = ok(&ws.call_with(&cx, &tool, json!({"path": "src"})));
    assert_eq!(
        text,
        "Outlines of the files in `src` (its own files; approximate): 2 of 3 files.\n\
         src/a.rs:\n  1  fn a() {}\nsrc/b.rs:\n  1  fn b() {}\n\
         [1 file with no definitions, or binary, not shown.]\n\
         [Skipped 1 entry that the permission rules do not let this call read.]"
    );
    assert!(
        !outliner
            .asked
            .lock()
            .unwrap()
            .iter()
            .any(|name| name == "secret.rs")
    );
    let deep = ok(&ws.call_with(&cx, &tool, json!({"path": "src", "depth": 2})));
    assert!(deep.contains("src/deep/c.rs:\n  1  fn c() {}"), "{deep}");
    assert!(
        !deep.contains("hidden") && !deep.contains("ignored"),
        "{deep}"
    );
}

#[test]
fn a_large_directory_is_cut_at_the_output_limit_and_says_how_many_are_left() {
    let ws = Workspace::new("outline_dir_limit");
    for n in 0..60 {
        ws.file(&format!("m{n:02}.rs"), &"fn f() {}\n".repeat(5));
    }
    let (cx, _) = with_outliner(&ws);
    let cx = cx.with_limits(OutputLimits::new(4096, 40, 256));
    let text = ok(&ws.call_with(&cx, &OutlineTool::new(), json!({"path": "."})));
    assert!(
        text.contains("more files not shown: the output limit was reached"),
        "{text}"
    );
    assert!(text.lines().count() <= 40);
}

#[test]
fn a_file_is_read_and_a_directory_walked_as_its_plan_says() {
    let ws = Workspace::new("outline_plan");
    ws.file("src/a.rs", "fn a() {}\n");
    let tool = OutlineTool::new();
    let file = ws.plan(&tool, json!({"path": "src/a.rs"})).unwrap();
    assert!(
        matches!(file.effects.as_slice(), [Effect::Read(path)] if path.display() == "src/a.rs")
    );
    assert!(!file.has_side_effects());
    let dir = ws.plan(&tool, json!({"path": "src"})).unwrap();
    assert!(matches!(dir.effects.as_slice(), [Effect::ReadTree(path)] if path.display() == "src"));
    assert_eq!(tool.spec().name, "outline");
}

#[test]
fn without_an_index_the_tool_says_there_are_no_outlines() {
    let ws = Workspace::new("outline_none");
    ws.file("a.rs", "fn a() {}\n");
    let text = failed(&ws.call(&OutlineTool::new(), json!({"path": "a.rs"})));
    assert!(
        text.starts_with("Outlines are not available in this session"),
        "{text}"
    );
}

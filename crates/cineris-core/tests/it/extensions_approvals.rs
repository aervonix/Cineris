//! What an approval of an extension shows before its yes (the fixes of the
//! full review of step 28, its findings 11 and 16): that the programs it
//! runs get the user's environment, in the hook commands' own words, and
//! the whole text it adds to the system prompt, quoted and escaped. Each
//! surface that approves or describes one is held: the trust question,
//! `cineris permissions trust` (`Core::project_extensions`) and `cineris ext
//! inspect` (`Core::extensions`); `cineris ext add` is held in
//! `cineris-cli`'s own tests.

use std::path::Path;

use cineris_core::ext::PROMPT_SHOWN;
use cineris_ext::hook::command::ITS_ENVIRONMENT;
use cineris_ext::manifest::MAX_PROMPT_BYTES;

use crate::World;
use crate::extensions_found::{NAME, open_answering, projects};
use crate::extensions_trust::{listed_detail, not_now};

/// Writes `probe` in `folder`: running `gh` when `runs`, and adding
/// `prompt` to the system prompt.
fn write_prompted(folder: &Path, runs: bool, prompt: &str) {
    std::fs::create_dir_all(folder).unwrap();
    std::fs::write(
        folder.join("extension.toml"),
        format!(
            "api = 1\nname = \"{NAME}\"\nversion = \"1\"\ndescription = \"A test \
             extension\"\nmain = \"main.js\"\nprompt = \"prompt.md\"\n[capabilities]\n{}\n",
            if runs { "exec = [\"gh\"]" } else { "" }
        ),
    )
    .unwrap();
    std::fs::write(folder.join("main.js"), "// nothing\n").unwrap();
    std::fs::write(folder.join("prompt.md"), prompt).unwrap();
}

/// A prompt file of the most the manifest takes, 4 KiB: one line that
/// would end the quote and pose as the harness, one with an escape
/// sequence, and filler, ending with a mark that is seen only when the
/// text is shown whole.
fn longest_prompt() -> String {
    let head = "Always \"close\" the quote `here`.\nClear \u{1b}[2J the screen.\n";
    let tail = "THE-LAST-WORDS";
    let filler = "x".repeat(MAX_PROMPT_BYTES - head.len() - tail.len());
    let text = format!("{head}{filler}{tail}");
    assert_eq!(text.len(), MAX_PROMPT_BYTES);
    text
}

/// What an approval shows of the longest prompt: after [`PROMPT_SHOWN`] on
/// a line of its own, quoted so that it cannot close its quote, escaped,
/// and whole.
fn shows_the_prompt_whole(said: &str) {
    let at = said
        .find(&format!("{PROMPT_SHOWN}\n\""))
        .unwrap_or_else(|| panic!("no prompt: {said}"));
    let shown = &said[at..];
    for part in [
        "Always \\\"close\\\" the quote \\`here\\`.",
        "Clear \\u{1b}[2J the screen.",
        "THE-LAST-WORDS\"",
    ] {
        assert!(shown.contains(part), "{part:?} is not in {shown:?}");
    }
    assert!(!said.contains('\u{1b}'), "{said:?}");
}

/// The trust question and `permissions trust` show a project's extension
/// with the environment its programs get and the whole text it adds to
/// the system prompt, at the 4 KiB the manifest allows; one that runs no
/// program says nothing of the environment.
#[test]
fn a_projects_extension_is_approved_seeing_its_environment_and_its_prompt() {
    let world = World::new("approvals_project_prompt", []);
    write_prompted(&projects(&world), true, &longest_prompt());
    let core = world.core();
    let detail = world.run(async {
        let (front, asked) = open_answering(&world, &core, not_now()).await;
        front.close().await;
        listed_detail(&asked)
    });
    assert_eq!(detail.matches(ITS_ENVIRONMENT).count(), 1, "{detail}");
    shows_the_prompt_whole(&detail);
    // `cineris permissions trust` shows the question's own text.
    let project = core.project(&world.workspace).unwrap();
    let listed = core.project_extensions(&project).unwrap();
    let [one] = listed.as_slice() else {
        panic!("one extension: {listed:?}");
    };
    assert_eq!(one.detail, detail);
    // Running no program, nothing is said of the environment.
    write_prompted(&projects(&world), false, "Short.");
    let quiet = core.project_extensions(&project).unwrap();
    let [one] = quiet.as_slice() else {
        panic!("one extension: {quiet:?}");
    };
    assert!(!one.detail.contains(ITS_ENVIRONMENT), "{}", one.detail);
    assert!(
        one.detail
            .ends_with(&format!("\n{PROMPT_SHOWN}\n\"Short.\"")),
        "{}",
        one.detail
    );
}

/// `cineris ext inspect` shows the same of a project's extension, trusted or
/// not, and of the user's own.
#[test]
fn ext_inspect_shows_an_extensions_environment_and_its_prompt() {
    let world = World::new("approvals_inspect_prompt", []);
    write_prompted(&projects(&world), true, &longest_prompt());
    let users = world.dirs().config.unwrap().join("extensions").join("mine");
    write_prompted(&users, true, &longest_prompt());
    // The user's is named for its folder.
    let manifest = std::fs::read_to_string(users.join("extension.toml")).unwrap();
    std::fs::write(
        users.join("extension.toml"),
        manifest.replace("name = \"probe\"", "name = \"mine\""),
    )
    .unwrap();
    let core = world.core();
    let project = core.project(&world.workspace).unwrap();
    let listing = core.extensions(&project).unwrap();
    for name in [NAME, "mine"] {
        let one = listing
            .extensions
            .iter()
            .find(|one| one.name == name)
            .unwrap_or_else(|| panic!("{name}: {listing:#?}"));
        let words = one.words.as_deref().unwrap_or_default();
        assert_eq!(words.matches(ITS_ENVIRONMENT).count(), 1, "{words}");
        let prompt = one
            .prompt
            .as_deref()
            .unwrap_or_else(|| panic!("{name}: no prompt: {one:#?}"));
        shows_the_prompt_whole(&format!("{PROMPT_SHOWN}\n{prompt}"));
    }
}

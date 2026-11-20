//! An extension's folder is read whole, holds only files and folders, and
//! stays within its caps.

use cineris_ext::folder::{FolderError, MAX_FILES, Snapshot};
use cineris_testkit::scratch;

use crate::make_link;

#[test]
fn a_folder_is_read_whole_and_its_hash_follows_its_content() {
    let scratch = scratch!("a_folder_is_read_whole_and_its_hash_follows_its_content");
    scratch.file("ext/extension.toml", crate::manifest(""));
    scratch.file("ext/main.js", "1");
    scratch.file("ext/lib/a.js", "2");
    let first = Snapshot::read(&scratch.join("ext")).unwrap();
    assert_eq!(
        first.paths().collect::<Vec<_>>(),
        ["extension.toml", "lib/a.js", "main.js"]
    );
    assert_eq!(first.file("lib/a.js"), Some(&b"2"[..]));
    // Read again unchanged, the same; any byte changed, another.
    assert_eq!(
        Snapshot::read(&scratch.join("ext")).unwrap().hash(),
        first.hash()
    );
    scratch.file("ext/lib/a.js", "3");
    assert_ne!(
        Snapshot::read(&scratch.join("ext")).unwrap().hash(),
        first.hash()
    );
    // A file added is a change too, hidden or not.
    scratch.file("ext/lib/a.js", "2");
    scratch.file("ext/.hidden", "");
    assert_ne!(
        Snapshot::read(&scratch.join("ext")).unwrap().hash(),
        first.hash()
    );
}

#[test]
fn a_link_anywhere_in_the_folder_refuses_it() {
    let scratch = scratch!("a_link_anywhere_in_the_folder_refuses_it");
    scratch.file("outside/secret.js", "stolen");
    scratch.file("ext/extension.toml", crate::manifest(""));
    scratch.file("ext/main.js", "1");
    scratch.dir("ext/lib");
    // A link to a file outside, deep in the folder.
    if !make_link(
        &scratch.join("outside/secret.js"),
        &scratch.join("ext/lib/x.js"),
        false,
    ) {
        return;
    }
    assert_eq!(
        Snapshot::read(&scratch.join("ext")),
        Err(FolderError::Link("lib/x.js".to_owned()))
    );
    std::fs::remove_file(scratch.join("ext/lib/x.js")).unwrap();
    // A link to a folder, even one inside the extension.
    if !make_link(&scratch.join("ext/lib"), &scratch.join("ext/again"), true) {
        return;
    }
    assert_eq!(
        Snapshot::read(&scratch.join("ext")),
        Err(FolderError::Link("again".to_owned()))
    );
}

#[test]
fn the_folder_itself_may_be_reached_through_a_link() {
    let scratch = scratch!("the_folder_itself_may_be_reached_through_a_link");
    scratch.file("real/extension.toml", crate::manifest(""));
    scratch.file("real/main.js", "1");
    if !make_link(&scratch.join("real"), &scratch.join("named"), true) {
        return;
    }
    let through = Snapshot::read(&scratch.join("named")).unwrap();
    assert_eq!(
        through.hash(),
        Snapshot::read(&scratch.join("real")).unwrap().hash()
    );
}

#[test]
fn a_folder_past_its_caps_is_refused() {
    let scratch = scratch!("a_folder_past_its_caps_is_refused");
    for index in 0..=MAX_FILES {
        scratch.file(format!("many/f{index}.js"), "");
    }
    assert_eq!(
        Snapshot::read(&scratch.join("many")),
        Err(FolderError::TooManyFiles)
    );
    scratch.file("big/one.js", vec![b'x'; 1024 * 1024 + 1]);
    assert_eq!(
        Snapshot::read(&scratch.join("big")),
        Err(FolderError::FileTooLarge("one.js".to_owned()))
    );
    for index in 0..5 {
        scratch.file(format!("total/f{index}.js"), vec![b'x'; 1024 * 1024]);
    }
    assert_eq!(
        Snapshot::read(&scratch.join("total")),
        Err(FolderError::TooLarge)
    );
    let deep: String = (0..20).map(|_| "d/").collect();
    scratch.file(format!("deep/{deep}x.js"), "");
    assert!(matches!(
        Snapshot::read(&scratch.join("deep")),
        Err(FolderError::TooDeep(_))
    ));
}

# Walking the workspace

Four places in the harness walk a directory tree with the `ignore` crate: the `search`, `list` and `outline` tools (one walk between them), the repository index's pass, and the look for changed files after a shell command. This page says which walker each uses and with what options, how they differ and why, and what the crate does differently between its two walkers, which cost step 18 a number of fixes.

The crate and its version are those of `Cargo.lock`: `ignore` 0.4.34, which walks with `walkdir` 2.5.0. Line numbers below are of those sources, read on 2026-11-15. Why the harness walks in-process at all is [decision 0014](../decisions/0014-search-in-process.md); the dependency is the `ignore` row of [dependencies.md](../dependencies.md).

## The two walkers of the crate

`WalkBuilder` makes either of two, from the same options.

| | `build()`: a `Walk`, an iterator on one thread | `build_parallel()`: a `WalkParallel`, run with a visitor on several |
|---|---|---|
| Used by | the index's walk in name order (the pass's fallback), the look after a shell command | `list`, `search` and `outline` (through `cineris-tools`' `walk`), the index's pass |
| Threads | one | the option `threads`; 0 is the available cores, at most 12 (`WalkParallel::threads`, `walk.rs` line 1537) |

Both walkers apply the same rules to the same entries: the ignore files, the hidden-file rule, `follow_links`, `max_depth` and the entry filter. They differ in four things that the options do not control.

### 1. Order

**The serial walker can sort; the parallel one cannot.** `sort_by_file_name` and `sort_by_file_path` are applied only in `WalkBuilder::build` (`walk.rs`, the `sorter` match at lines 612 to 628); `WalkParallel` has no sorter field (line 1411). The crate says so in the doc of both methods ("Note that this is not used in the parallel iterator"). The serial walk with a sorter is depth-first with each directory's entries in sorted order.

**The parallel walker has no order at all,** not even the file system's. Each thread keeps a last-in, first-out stack of work (`Stack::new_for_each_thread`, line 1663, `Deque::new_lifo`), a directory's entries are pushed in the order they were read and so are visited last first, and threads steal from each other. Checked on Linux with `ignore` 0.4.34 and one thread, over a tree with `a`, `b`, `denied`, `m` and `z` in its root: the serial walk with a name sorter gave `a`, `b`, `denied`, `m`, `z`; the parallel walk gave `denied`, `a`, `b`, `z`, `m`.

So a caller of the parallel walker sorts what it gathers, or keeps it in an ordered structure, and makes anything else it reports (counts, the first of several reasons) independent of the order. The tools do both (see "What each walk in the harness uses", below); the index sorts its files with `path_order` after the walk.

### 2. How an ignore file is found

**The serial walker opens an ignore file by its path; the parallel one looks for its name among the entries it has read.** For each directory the serial walker calls `Ignore::add_child` (`dir.rs` line 272; used at `walk.rs` line 1242), which has `create_gitignore` open `<dir>/.gitignore`, `<dir>/.ignore` and `<dir>/.git/info/exclude` and lets the file system say whether each exists. The parallel walker reads the directory first and calls `add_child_with_entries` (`dir.rs` line 313), which first runs `collect_ignore_files` (line 337) over the entries: it compares each entry's name with `.ignore`, `.gitignore`, `.git` and `.jj` byte for byte (line 353 for `.gitignore`), and opens only what matched.

On a file system that does not tell case apart, which is NTFS by default, `.GITIGNORE` is `.gitignore` to the serial walker, which opens it through the file system and finds it, and a different name to the parallel one, which does not match it and never opens it. The same goes for `.IGNORE` and `.GIT`. On Linux the two agree, since there the file system tells them apart too. **This could not be run here:** the check above is of the source and of the tests of step 18 (the two names are different files on Linux), and the Windows behaviour is not measured in this repository. Two limits to it from the source: the ignore files of the directories above the root of the walk are found by path in both walkers (`Ignore::add_parents`, `dir.rs` line 195, called by both), and so is a directory at `max_depth`, which the parallel walker does not read (`Work::add_ignore`, `walk.rs` line 1604).

The consequence for the harness: a listing made with the serial walker (the index's fallback at its cap) can differ from one made with the parallel walker (every other listing) in a workspace that has `.GITIGNORE` on a case-insensitive file system. `search` and `list` use the parallel walker, so the map's listing matches them below the cap, which is the reason the pass uses it. Tested: `index::an_ignore_file_is_known_by_its_exact_name_on_any_number_of_threads` (`cineris-index`) and, for both walkers on one tree, `walk::tests::the_walk_in_name_order_lists_what_the_walk_on_threads_lists`.

### 3. When the entry filter and the visitor are asked, and what has been read by then

**The parallel walker reads a directory whole before anything in it is judged; the serial one asks as it goes.**

- In the parallel walker, `run_one` (`walk.rs` line 1769) reads a directory with `Work::read_dir` (line 1815), which collects every entry and loads the ignore files, and only then calls the visitor with the directory itself (line 1821). Then `generate_work` (line 1874) is called for each of the entries in one loop (line 1841): it applies the ignore rules and the `filter_entry` predicate (line 1928) to each, and queues the rest. So **the predicate is asked about all the entries of a directory in one burst, on the thread that read it, before any of them is visited.** A visitor that stops the walk at one entry cannot keep the predicate from being asked about that entry's siblings.
- **A visitor that returns `WalkState::Skip` for a directory does not stop it being read.** The directory was read and its ignore files opened at line 1815, before the visitor saw it. Skip only keeps the walk from queueing what is in it (the early return at line 1823). Checked with `strace` on Linux: with the refusal in the visitor, the parallel walker opened the directory and its `.ignore`; with the same refusal as a `filter_entry` predicate, it opened neither.
- In the serial walker, `Walk::next` takes one event at a time from `walkdir`: for each entry it calls `skip_entry`, which applies the ignore rules and the predicate (line 1178), and for a directory the predicate refuses it calls `skip_current_dir` (line 1230). `walkdir` has opened a directory by the time it yields it, so a refused directory is opened, and with a sorter its entries are read to be sorted (checked with `strace`: the directory is opened and read, and its ignore files are not).

So the only way to keep a refused directory from being opened at all is the parallel walker with the refusal in `filter_entry`.

### 4. Threads, stopping and errors

- **Stopping.** The serial walk stops when the caller leaves the loop. In the parallel one a visitor returns `WalkState::Quit`, which makes the thread call `quit_now`; the other threads finish the item they have. Each visitor sees entries on one thread only, and the visitors run concurrently.
- **Errors.** An entry that could not be read reaches the visitor as an `Err`. In the parallel walker the errors of a directory's entries come after all its valid entries (`for err in readdir.errors`, line 1852); in the serial one they come in place.

## What each walk in the harness uses

| | `search`, `list`, `outline` | The index's pass | The look after a shell command |
|---|---|---|---|
| Code | `cineris-tools`, `walk::walk` | `cineris-index`, `walk::list` | `cineris-tools`, `shell::changes` (`modified_since`) |
| Walker | parallel; `threads` 0 for `list` and `outline`, `search`'s own setting (0 by default) | parallel on the pass's threads (half the cores, at most four) and, at the cap on entries, serial in name order | serial, no sorter |
| Ignore files | `.gitignore` at every level whether or not the tree is a repository (`require_git(false)`), `.ignore`, `.git/info/exclude`, and the ignore files above the root (`parents`) | the same | the crate's defaults with `require_git(false)`: the same ignore files |
| The user's global git ignore file | off (`git_global(false)`): it is the machine's | off | off |
| Hidden files | left out (`hidden(true)`: a name starting with `.`, and on Windows a file with the hidden attribute), so `.git` and `.cineris`. `list` with `include_ignored` turns the hidden rule and every ignore file off | left out | **included** (`hidden(false)`), except that a name `.git` is filtered |
| Links | not followed (`follow_links(false)`) and reported: `search` counts them, `list` shows them as what they lead to, `outline` does not name them | not followed, not listed, counted | not followed |
| Depth | `list` and `outline` as the call says; `search` unlimited | unlimited | unlimited |
| The permission gate | `ReadGate::may_read`, asked of every entry in the visitor: a directory it refuses is stepped past with `WalkState::Skip`, **after it has been read** | `Gate::may_index`: a directory in `filter_entry`, so a refused directory is not read on the parallel walk; a file in the visitor | none: it names changed files and reads nothing of them |
| Names the path rules refuse | stepped past, with what is under them, and counted | the same | a name that is not Unicode is not reported |
| Caps | `list`: `max_results` (200, at most 5,000) and the output limit, kept as the walk goes; `outline` keeps 2,000 files; `search`: `max_results` (2,000 at most). The walk itself is not capped | 100,000 entries listed, 20,000 files kept (the shallowest), 1 MiB a file | 20,000 files and half a second (`DEFAULT_CAP`) |
| Order of what is reported | `list` and `search` in the byte order of the path as written; `outline` name by name (`a/x.rs` before `a-b.rs`) | name by name (`path_order`) | sorted by path as text |
| Who stops it | the call's cancel token, looked at before each entry | the session's end or a turn that begins, looked at before each entry; never the time | the cap |

The sources are `walk.rs` of both crates, `list.rs`, `search.rs`, `outline.rs` and `shell/changes.rs`. The shared rules are in [05-tools.md](05-tools.md) ("A walk is checked twice", "A directory walk names every entry through the same resolver", "A walk does not follow links") and the index's in [15-efficiency.md](15-efficiency.md), "Repository map".

## Why they differ

- **The tools walk on threads for speed on large trees** ([decision 0014](../decisions/0014-search-in-process.md)). They sort after, since the parallel walker cannot. They keep only what a result can show while the walk runs (`list` and `outline` in a set that drops its last entry past the limit, `search` in a map that does the same), so the cost does not grow with the tree.
- **The tools ask the gate in the visitor** because `filter_entry` takes a `'static` predicate (`Fn(&DirEntry) -> bool + Send + Sync + 'static`, `walk.rs` line 1046) and a tool's `ReadGate` is borrowed from the call. The price is section 3: a refused directory is read before it is refused. What it exposes is the directory's entries and its ignore files to the harness, which uses neither; nothing of it reaches the model or a result's count but the number of entries stepped past.
- **The index's gate is an `Arc`,** which a `'static` predicate can hold, so it is asked in `filter_entry` and a refused directory is never opened.
- **The index sorts its listing, and falls back to a serial walk in name order,** because a listing cut at its cap must be the same one on every machine, and which entries the threads reach first changes from one run to the next. The fallback is the only place the serial walker is used for a listing, and so the only place section 2 shows (the comment of `walk::list` in `cineris-index`, and the answers of [step 18](../steps/18/answered.md)).
- **The look after a shell command is not a listing,** and reads nothing of what it names. It takes hidden files because a command may change any of them, and it has no gate for the same reason.

## What a later step must do to keep them consistent

1. **A change to the options goes into both walks' builders.** `cineris-tools`' `walk::walk` and `cineris-index`'s `walker` set the same eight options (`follow_links`, `hidden`, `ignore`, `git_ignore`, `git_exclude`, `parents`, `require_git` and `git_global`) separately. A new option, or a new ignore file name, is made in both, and in the doc of [05-tools.md](05-tools.md) and [15-efficiency.md](15-efficiency.md). There is no shared builder: neither crate depends on the other ([01-workspace.md](01-workspace.md)), and a home for one below both, `cineris-base`, would give that crate the `ignore` dependency.
2. **Never rely on an order from the parallel walker, and never sort before the walk and expect it kept.** A sorter on a builder given to `build_parallel` is silently ignored. Sort after, or keep the results in an ordered structure.
3. **Use the parallel walker for any listing that must agree with `search` and `list`,** and the serial one only where its order is needed, with the difference of section 2 in mind: it honours an ignore file by a name of another case on a file system that does not tell case apart. If a serial listing must agree with the parallel one, the `.GITIGNORE` case needs a rule of its own (for instance, comparing names exactly before opening an ignore file), or the serial walker is not used.
4. **A refusal that must not open a directory goes in `filter_entry`,** on the parallel walker, and needs an `Arc` it can hold. A refusal in the visitor, as the tools make it, steps past what is in the directory and nothing more. A step that adds a protected-path rule to the tools' gate should say, where it documents the rule, that the directory is read first.
5. **Test with the case the walkers differ on.** A test of a new walk puts an ignore file in nested directories, one named in another case, a refused directory with an ignore file in it, and a tree that the cap cuts, and compares the listing on one and on several threads (the index's tests do, as [step 18](../steps/18/answered.md) recorded). On Linux the `.GITIGNORE` case cannot differ, so it needs the Windows run: `gh workflow run try --ref <branch> -f os=windows -f command="test -p <crate> <filter>"`.
6. **Keep the orders in view.** `list` and `search` sort the path as text (`a-b.rs`, `a/`, `a/x.rs`); `outline` and the index sort name by name (`a/`, `a/x.rs`, `a-b.rs`). They differ for any name that holds a character below `/` (a space, `-`, `.`, `+` and some others). The recommendation, if one is to be chosen for all, is the name-by-name order, which keeps a directory's files together; the change is made in `list` and `search` in one step with their pages.
7. **When `ignore` is upgraded,** read the source again for these three: the sorter in `build_parallel`, `collect_ignore_files`, and where `read_dir` falls in `run_one`.

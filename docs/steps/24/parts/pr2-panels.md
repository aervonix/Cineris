# Step 24, its second pull request: the panels

The second of step 24's three pull requests: the panel mechanism, and the plan and review panels, in `cortex-tui` alone. It was built by a session of its own beside the first pull request (`config/reload` and the protocol's additions), and the third (the usage and project panels, the commands, the reloads) builds on it. What it built is described in [09-tui.md](../../../architecture/09-tui.md), "The panels as built".

## What was built

| Where | What |
|---|---|
| `app/ready.rs` | `Readiness`: the dialog's rule of when keys are taken (the grace, the quiet, the time named, the answers drawn), taken out of `app/dialog.rs` so that the panels hold to the same one. The dialog keeps its methods and its behaviour, and every one of its tests passes unchanged. |
| `app/panels.rs` | The mechanism: the `Panel` trait (one type and one registration a kind), the open panels in the order they opened, their layout in the live region's limit (the focused one scrolled with `… N more` rows, the oldest collapsed ones folded into `+N panels · ctrl+p`), what can be focused, the focus kept on its item by id, the readiness of each, and how many of its operations are on their way. |
| `app/panels/plan.rs` | The plan panel: `plan.updated`, `plan/get` for a stored session, and `plan/update` for each key (skip, reword, add after, delete, reorder). |
| `app/panels/review.rs` | The review panel: the files a turn's file tools changed, with their counts and the viewer's box of the last call, whether a command ran, the verification notices, the verdict. |
| `app/state/panel.rs` | The application's side: the panels' block in the frame, the keys (readiness, the held key, the typing rule, a change on its way), the operations sent with the session's id, `/plan`, `App::set_keymap`, the verdict in the turn summary. |
| `input.rs` | `Outcome::Panel`, the item edit (`begin_item_edit`, `Outcome::ItemEdited`, `Outcome::ItemEditCancelled`), `set_keymap`, and the typing rule (`pass_to_editor`): a key that types and is not taken with a box or a panel focused goes to the editor, and the focus with it. |
| `keys/mod.rs` | The focus context's thirteen panel keys, `focus.skip` to `focus.close`, and `PANEL_KEYS`. |
| `focus.rs`, `transcript.rs`, `app/items.rs` | `Ring::focus`, `Transcript::ends_blank`, `Mapper::round_of`. |
| `view/panel.rs`, `view/plan.rs`, `view/review.rs` | The panels' rows. Step 22's view functions are not changed. |
| Screen 4 | The plan's rows and the block's blank row are compared (`COMPARED` 36 to 42); the usage panel's limit line stays not built. In `4-light.ans` one cell changed, row 33 column 2: the done item's `☑` is bold, as the light theme draws the success colour since the owner's colour decision of 2026-11-15, which rewrote the other screens when these rows were not compared. |

## The interfaces the third pull request builds on

- `app::panels::Panel` (`pub(crate)`, `Debug + Send`): `name`, `seen(&Seen) -> Show`, `result(&OpResult) -> Show`, `when_opened(&SessionId, stored) -> Option<Op>`, `summon() -> Result<(), String>`, `collapsed(&Draw)`, `expanded(focused, &Draw) -> Vec<PanelRow>`, `items`, `item(i) -> Option<ItemKey>`, `first_focus`, `least_rows(focused, &Draw)` (a default that counts the rows; a costly kind says it without drawing them), `hint(focused, &KeyMap)`, `act(Action, focused, Option<&SessionId>) -> Act`, `activate(focused) -> Act`, `edited(&ItemEditFor, &str, Option<&SessionId>) -> Act`. A kind is registered in `state/panel.rs`, `registered()`.
- `Seen::{Event { event, round, running }, Prompt, Left}`, `Show::{Same, Open, Close}`, `Act::{Pass, Taken, Send(Op), Edit { text, hint, edit }, Close, View(Option<usize>), Say(String)}`, `Draw { width, look, tick }`.
- `Input::begin_item_edit(text, hint)` (`hint` is what `Enter` does: `saves the item`), `item_edit()`, `cancel_item_edit()`, `pass_to_editor(key)`, `set_keymap(keymap)`; `Outcome::Panel(Target, Action)`, `Outcome::ItemEdited(String)`, `Outcome::ItemEditCancelled`.
- A reload of `[keys]` calls `App::set_keymap`, not `Input::set_keymap`: only it starts the panels' readiness (and the dialog's) again.
- `app::ready::Readiness` (`pub(crate)`), with `GRACE`, `QUIET` and `REPEAT`, which `app::dialog` names too.

## Departures, and the choices made

1. **The focus ring is given the panels that would be drawn focused,** not only those drawn: a folded panel is reached with `Ctrl+P`, as its row says, and the invariant holds that a focused panel is drawn with its focused row. The folded row names the key only when a panel folded there can be reached (since the review).
2. **A blank row goes above the block** when no live row is above it, so that it does not touch the scrollback; `Transcript::ends_blank` keeps it from doubling one already there.
3. **A skipped item counts as done** for closing the plan: a plan of done and skipped items has nothing left to do.
4. **The review opens at the turn's first verification notice,** so that `checks running · round 1 of 2` is seen; and when the turn ends having changed files.
5. **The review closes at any `turn.started`** (another client's turn, a queued prompt's), and when this client sends a prompt that starts a turn.
6. **An empty item, or a reword to the same text, sends nothing.**
7. **The turn's verdict rides in the turn summary's cost text** (`$0.42 · checks passed`), muted, a skipped verdict saying nothing: the summary's view takes no verdict, and colouring it would change step 22's view.
8. **While an item is added, the empty input box shows the prompt's placeholder,** and `view::input`'s comment that typing would not reach a focused panel is now out of date: both are step 21's and 22's view code, left as they are; the hint row says `enter saves the item · esc cancels`.
9. **A panel's key again within a second of one the panel took is that key held,** and goes nowhere: two deliberate presses of one key so close are taken for one.
10. **`plan/get` is asked only of a stored session** (resumed, switched to, forked): a new one has no plan.
11. **`Enter` acts at once,** as moving does: on a review's file it opens the viewer, which changes nothing.
12. **While a change of a panel's is on its way, a key's change is not sent** (since the review): the key is taken and does nothing, rather than computed from what the answer will change. With the core in process the answer comes before the next key; over a link with a delay a quick second edit is lost, and is pressed again.

## The independent review, and what each finding changed

The whole branch was reviewed before its first push. The reviewer found sound: the layout's bounds, no quadratic work, the sanitising, the handling of sessions left, the plan's revisions, the viewer's box numbers, the dialog's refactor, about twenty breaks traced, step 22's views untouched, and the change to screen 4.

| Finding | What changed | Test | Break |
|---|---|---|---|
| 1. A change to the focused item from outside started the readiness again only at the next frame, and the loop takes keys between what it drains: a key read in between acted on the item the focus moved to | The change is seen where it lands (`Panels::follow`, after every event and result), and the readiness starts again then | `a_key_read_after_the_focused_item_went_and_before_a_frame_does_not_act` | `a-changed-focused-row-keeps-the-readiness`, moved to `Panels::follow` |
| 2. An item edit could carry a line break or a tab (a paste, `Ctrl+G`), which the core refuses, losing the text | The plan sends control characters as spaces | `a_pasted_line_break_in_an_item_edit_saves_one_line` | `an-item-edit-sends-its-line-breaks` |
| 3. The folded row offered `ctrl+p` when nothing behind it could be reached, and `/plan` in a window too small took the focus for nothing | The key is named only when a folded panel is reachable; a panel a command asked for that cannot be drawn focused says the window has no room | `slash_plan_in_a_window_with_no_room_for_it_says_so`; the fold test checks `reachable` | `the-folded-row-offers-a-key-that-reaches-nothing`, `slash-plan-with-no-room-says-nothing` |
| 4. A session that failed to open replays its held notices, and a verification notice opened the review for it | Only the events of the session open reach the panels | `a_notice_of_a_session_that_did_not_open_opens_no_panel` | `a-notice-of-a-session-not-open-reaches-the-panels` |
| 5. A held `x` closed the review and then typed `xxxx` into the prompt | The held-key rule is asked before anything has the focus (`App::panel_holds`) | `a_held_x_that_closed_the_review_types_nothing` | `a-held-key-types-once-its-panel-lost-the-focus`; `a-held-panel-key-acts-again` moved to `panel_holds` |
| 6. A reorder was computed from a plan an earlier edit was about to change | While a panel's operation is on its way, its keys send nothing (departure 12) | `an_edit_on_its_way_holds_the_next_until_it_is_answered` | `an-edit-on-its-way-lets-the-next-be-computed-from-an-old-plan`, `an-answer-does-not-end-the-wait` |
| 7. The frame's hostile-text test passed with no panel drawn | It checks both panels' first rows and counts the rows with the escaped text, for each focus | `hostile_plan_items_and_paths_are_drawn_visible_in_the_frame` | `the-panels-block-is-not-drawn` |
| 8. This part had no record | This page; 09-tui points to it | | |
| 9. Small points | The review view's example drew `exit 0` on a passed check, which it never draws: corrected. A sort of `reachable` that did nothing: gone. As-built said the keys arrive as `Outcome::Panel` while the application looked them up itself: it now takes them from the input's `Outcome::Panel`. The plan collapsed during its own item edit: it stays drawn whole, the item shown. | `the_plan_stays_drawn_whole_while_its_item_is_edited` | `the-plan-collapses-while-its-item-is-edited` |

## Tests

All tier-1 unit tests of `cortex-tui`, with no real waiting (the application's tests give every event its time):

- `app/tests/panels.rs`: each panel opens and closes on its conditions, edges included; every plan key's `plan/update`, a result replacing the plan, an error said, a result for a session left dropped, `plan/get` for a stored session; the readiness (typing past `/plan`'s `Enter`, held keys marked and unmarked, keys read together, a paste, moving and `Esc` at once, the grace again after the viewer, `fg`, the external editor, a dialog, a changed row and a new key map, a key between a dialog and its frame); the typing rule with a box and a panel; the review's checks as they come, `x` and `Enter`; the verdict in the summary; hostile text in the frame; and the review's findings above.
- `app/panels/tests.rs`: the sweep of the live region (every height from 4 to 60 rows, every set of the plan, the review and two stand-in panels of other shapes, each focused or none, with a round running or not, and with a menu open: the region within `max(rows / 2, bottom)`, a focused panel drawn with its focused rows or the input focused, no row wider than the width), in two tests; scrolling, folding, what can be reached, the window, `least_rows` against the rows drawn, the blank rows and the four live rows kept.
- `input/tests.rs`, `view/plan/tests.rs` (screen 4's rows cell for cell, every status's tokens), `view/review/tests.rs`, `view/panel/tests.rs`, `app/ready.rs`, `focus.rs`; snapshots of both panels at 60, 80 and 120 columns, in both glyph sets, with colour and without, light and dark (`snapshots/plan.txt`, `snapshots/review.txt`).
- Extended: `tests/it/bindings.rs` (each panel key, focused panel and focused box) and `tests/it/decoder.rs` (`space` as a win32 input record).

## Breaking the code on purpose

`mutations/step-24-panels.breaks` holds 59 breaks, run with `cargo xtask mutate step-24-panels --jobs 2` on the desk's side slot: **58 caught, and 1 survives as its list expects** (`the-viewer-keeps-the-panels-readiness`: every key that closes the viewer is input no panel took, which holds the panel a quiet second, longer than the grace this break would leave).

- The first run flagged 5. Three were caught by other tests than their lists named, and the names were put right. Two showed rules untested: a key read after a dialog came and before its first frame (`a_key_between_a_dialog_and_its_frame_is_typed_not_lost`), and the box's typing rule, whose test began with a panel's own letter (`a_letter_with_a_box_or_a_panel_focused_reaches_the_editor` now begins with one that is not).
- After the review's fixes one was flagged: `a-held-panel-key-acts-again` was caught only by the new held-`x` test, because the hold for a change on its way dropped the held key's repeats first. Its own test now answers the press before the repeats come, and catches it again.
- Taking the dialog's rule into `app/ready.rs` moved code out from under 11 of step 23's breaks (`step-23.breaks`, `step-23-fixes.breaks`). Each was moved to the code as it is now, and each is still caught. Two of this part's own would have made the same changes as two of those, and were dropped: step 23's carry those rules.

## Stress

- `cargo xtask stress -p cortex-tui --count 50`: 470 tests, 50 times each, seeds 0 to 49: every test passed every time.
- `cargo xtask stress -p cortex-tui --count 50 --cores 2 --load 6`: the same, every test passed every time.

## Tiers

- **The sweep of the live region** first took 1.3 s at the desk in the `dev` profile: the application's own frame (the transcript's live round and the hint row, step 22's views) is most of a frame's cost, and the sweep draws about 5,500 frames. It is drawn at 40 columns (where the hint row is short) with one call live, the edits' round and the checks' notice committed, and is two tests, one with a round running and one without: **0.25 s and 0.40 s in CI's profile at the desk** (0.24 s and 0.38 s before the review's fixes).
- **What this part's tests add:** 1.64 s of `cortex-tui`'s 7.08 s of tier-1 test time (23%), summed over the tests in CI's profile at the desk; nothing measurable to tier 2 (the bindings' and the decoder's loops take the new keys).
- **The estimate for a hosted runner** (`cargo xtask ci --tiers --like-runner -p cortex-tui`), the whole crate: tier 1 4.7 s on Linux (6.8 s on a slow runner) and 6.1 s on Windows (7.4 s), tier 2 0.8 s on Linux, of budgets of 20 s and 40 s for the whole workspace. So this part adds about 1.1 s to tier 1 on Linux and 1.4 s on Windows.

## Checks

- `cargo xtask ci` on 327cfaa4 with this part on top: formatting, the breaks, the generated files, clippy, licences, tiers 1 and 2 (4030 tests) and the tier budgets pass. **`rules` fails on this record alone,** and so does the system test that runs the rules (`xtask rules::tests::sys_the_repository_follows_its_own_rules`): no record of step 24 on this branch links `parts/`. The step's `answered.md`, which does, is the first pull request's; with it the record is linked.
- `cargo xwin clippy --target x86_64-pc-windows-msvc -p cortex-tui --all-targets --locked -- -D warnings`: clean.
- `cargo xtask api-diff --against origin/main --features cortex-tui/test-kit -p cortex-tui`: 57 public items added, none removed or changed; the test kit is unchanged. `App` stays `Send` (a panel is `Send`).

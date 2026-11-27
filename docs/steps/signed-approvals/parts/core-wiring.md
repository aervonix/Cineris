# Signed approvals, part: the core checks what it uses

The part of the first pull request ([answered.md](../answered.md), "The pull requests") that wires `cineris-policy`'s signed stores into the core: every read the core makes of `pins.toml` and `trust.toml` passes the key the person opened their approvals with, what restricts holds whatever its signature, the trust question carries what a yes would write and takes only a signed yes, and the command line's approving calls of the core sign with the person's key. The password prompt, the command line's signing at a terminal and the application's dialog are the parts after it.

## What was built

- **Every read passes the session's key** (`Core::verifier`, the public half of the key the person opened their approvals with, `None` until they have).
  The extension pipeline's inputs carry it (`ext::found::Finding::verifier`), as do `assemble::trust` and the trust question, a reload's judgement, `extensions/status`, `cineris ext list`, a build without the script engine (`without_engine::SessionView`), and the core's management of a project's trust.
  The pipeline's finding keeps the key it was opened with for the session: approvals opened later count from the next session.
- **The restriction side** (the maintainers' answer to call 4: Esc and headless runs keep every gate).
  - **The user's pins that do not count** (no key, unsigned, or not the person's) are taken as a pin store that cannot be read: each of the user's extensions found is not loaded, and holds the gates of the registration cache's entries of its name beside a gate on each gating event its manifest lists. The session is told once, in words for each case, and each extension's refusal says why. One the session loaded and that is not found keeps the gates it registered, as under an unreadable store.
  - **A project's entry that does not count** (`has_entry` and not `is_trusted`) leaves the project not trusted, and holds the gates of what it pins (`ProjectTrust::held`): each project hook command it pins is on its event, failing, through `ext::hooks::in_session`'s refused list; each extension it pins holds its gating hooks through the pipeline's pending gates, from the cache's entry at the hash pinned, or both gates where it has none (`ext::found::Uncounted`; `without_engine::uncounted_gates` in a build without the script engine). A project with no entry holds nothing. The session is told once, as it opens (`person::project_uncounted`). Under `--trust-project` too, since the review's fixes ([review-fixes-core.md](review-fixes-core.md)): as built here the entry was not read there, and its gates lapsed.
  - **The words:** "your approvals are not open in this session"; "written by an older version of Cineris, or before your password was reset: asked again"; "not signed by your key: changed by something other than Cineris" (`person::uncounted`). The middle one says the reset too, since a reset leaves the stores unsigned as an older version does.
  - **The registration cache is pruned** of the hashes no pin names, signed or not (`pins(None).held()`), so that a pin that does not count keeps the entry its gates are held from.
- **The trust question** (`assemble::look`, `TrustQuestion::proposal`, `Core::trust_for`).
  - **Asking.** When the question is put, the change a yes makes is worked out against the entry as it is (`TrustStore::propose`): `Trust { lift_refusal: false }` for a first question, with or without the approvals open, `Approve` for what is new in a trusted project. The request carries the proposal's view and its summary, and the question keeps the proposal. A store that cannot work it out (its project's trust taken away since the reading the question is shaped by, or unreadable now) puts no question, and the session is put together with the trust as it stands then.
  - **Not asked:** a project whose entry does not count only because the approvals are not open: its trust is there, and the password is what is missing.
  - **Answering.** A yes to a question with a proposal is checked where it is given (`clients::trust_answer_problem`): no signature, one that is not 128 lowercase hexadecimal digits, the approvals not open, or a signature that is not the open key's over the statement rebuilt from the proposal (`signed::statement_of`) is refused `invalid_answer`, with a message, and the question stays open. A taken yes is committed with the open key: `Moved` puts the question again with a fresh proposal (`assemble::Answered::AskAgain`, a loop in `trust_for`, of three rounds at most since the review's fixes); `NotSigned`, or no key or signature by then, trusts nothing, not even for the session, and says so; a never said meanwhile is not lifted, as before; only a signed yes the store could not write trusts the project for the session alone. A keep and a never stay unsigned.
- **`TrustStore::propose` takes `Option<&Verifier>`** (`cineris-policy`): a trust is worked out with no key (its summary then says the earlier trust is replaced, without saying whose it was); an approval and a drop are refused without one. `PinStore::propose` is unchanged.
- **The command line's approving calls of the core sign** (`Core::trust_project`, `trust_project_with`, `add_extension`, `remove_extension`).
  Each takes the person's key: proposes the change, signs it, commits it with the key's verifier, and opens the core's approvals with the key. `add_extension` pins `Approved { definition, registration: None }` (binding the registration hash is the second pull request's). A removal of a pinned extension is a signed `Unpin`; one with nothing pinned, signed or not, signs nothing, so `remove_extension` takes `Option<&PersonKey>`. `extension_removal` and `disable_extension` find the pin among every pin held, signed or not.
  `cineris-cli`'s callers get the key from a stub, `commands::person_key`, which refuses with "signing at a terminal comes with the CLI part of the signed approvals step: nothing was written"; the CLI part replaces it with the terminal's prompt. Since the review's fixes each of the four first refuses, writing nothing, a key that does not open the key file or is not the open key (`Core::signing_key_refusal`).
- **`Core::reset_approvals_at`**, hidden: a reset at a given cost, for tests, whose unoptimised builds take seconds at the standard cost. `reset_approvals` is it at the standard cost; nothing of the product calls it otherwise.
- **The tests.** Every core test that pins or trusts signs as the person (`cineris_policy::test_support::Person`), and every world's core opens its approvals with that person's key (`World::core`; `World::closed_core` for one nobody opened). A trust question's yes is signed by the test's front end over the request's proposal (`crate::signed`, `yes_to`, `signed_by`). The policy crate's integration tests of `pins.toml` and `trust.toml` sign too, and say what the versions before read as.
- **The breaks:** [mutations/signed-approvals-core-wiring.breaks](../../../../mutations/signed-approvals-core-wiring.breaks), seven.

## What the first pull request closes, and what it leaves unsigned

**Closed by it** (with the parts beside this one): the user's pins (`pins.toml`) and each project's trust (`trust.toml`) count only signed by the person's key, and what they pin restricts whatever its signature; commands start without the harness's console on Windows ([windows-console.md](windows-console.md)); the harness's process is closed to the user's other programs ([process-hardening.md](process-hardening.md)); and the managed configuration file is the administrator's ([managed-config.md](managed-config.md)).

**Unsigned until the second pull request**, as before the step ([answered.md](../answered.md), "The answers"), and so still written to effect by a command with the user's rights where nothing confines it:

- **stored approvals** (`approvals/<key>.toml`, "always allow in this project"), which allow what they name;
- **the accepted entries of what was learned** (`projects/<key>/knowledge.toml`), which run a learned command unasked, a check at a turn's end, or put a fact in the prompt;
- **the registration cache** (`extensions/registered.json`), from which a session takes what an approved extension registered at its pin's hash: an entry written there that fits the extension's manifest can leave out hooks its script registers, dropping a gate it held. The second pull request binds the cache's entry to the hash in its pin.

`docs/security.md`, section 3, says the same.

## Each rule and the test that holds it

| Rule | Test |
|---|---|
| Pins signed by another key approve nothing, and the extension holds its gate | `cineris-core::it signed::an_extension_pinned_with_another_key_is_not_loaded` |
| A pin taken out by hand lifts no gate | `signed::a_pin_taken_out_by_hand_lifts_no_gate` |
| A project trusted by another key, or an entry copied from another project, is not trusted, and its pinned extension holds its gate | `signed::a_project_trusted_with_another_key_or_copied_from_another_is_not_trusted` |
| Without the key, a pinned extension holds its gates | `signed::a_pinned_extension_holds_its_gates_without_the_key` |
| Without the key, a project's pinned hook holds its gate, the session is told once, and nobody is asked | `signed::a_projects_pinned_hook_holds_its_gate_without_the_key`; `cineris-core ext::hooks::tests::a_hook_an_entry_that_does_not_count_pins_holds_its_event` |
| A project with no entry holds nothing | `signed::a_project_with_no_entry_holds_no_gate` |
| An unsigned yes, a yes signed by another key, and a yes with the approvals closed are refused, the question open; the person's signed yes trusts | `signed::a_yes_trusts_only_signed_by_the_person`, `signed::an_unsigned_or_wrongly_signed_yes_trusts_nothing` |
| A version 1 `pins.toml` and a version 2 `trust.toml` approve nothing, and keep their gates | `signed::an_older_pin_store_approves_nothing_and_keeps_its_gates`, `signed::an_older_trust_store_trusts_nothing_and_keeps_its_hooks_gate`; `trusting::a_store_of_an_older_version_is_asked_about_again` |
| A reset lifts no gate | `signed::a_reset_lifts_no_extension_gate`, `signed::a_reset_lifts_no_hook_gate` |
| A signed yes whose entry moved is asked again | `trusting::a_yes_after_the_projects_trust_changed_meanwhile_is_not_used` |
| A trust is proposed without a key; an approval or a drop is not | `cineris-policy trust::tests::a_trust_is_proposed_without_a_key_and_an_approval_or_a_drop_is_not` |
| Pins count only with the key they are signed by | `cineris-policy::it pins::pins_count_only_with_the_key_they_are_signed_by` |

## What ran

On the reference machine, its disk stalling for much of each run:

- `cargo xtask fmt`, `cargo xtask schema` (the protocol's schema and TypeScript declarations regenerated and committed), `cargo xtask clippy` of `cineris-core`, `cineris-policy`, `cineris-cli` and `cineris-rpc`, `cargo xtask check no-scripts` and `cargo xtask mutate --check`: clean.
- `cargo xtask test --dependents -p cineris-policy` (`cineris-policy`, `cineris-core`, `cineris-ext`, `cineris-cli`, `cineris-rpc`): 1,416 passed, 11 failed, every failure one the parts after this one settle (below). Every `cineris-core` and `cineris-policy` test passes, the new ones included; one that ran out of time under the stall (`extensions_reload::with_the_examples_approved_no_engine_starts_until_one_is_used`) passed in 1.2 s run alone.
- Each of the seven breaks made by hand, one at a time: every one fails the tests its `caught-by` names. `cargo xtask mutate` itself did not run (no long run on the shared machine).
- `cargo xtask check`, once: formatting, the break lists, the generated files and the licences passed; `rules` failed (below); its workspace clippy and its build without the script engine failed on two findings of this part, fixed since and checked again crate by crate above.
- Not run: CI, Windows among it; the interactive application's tests (`cineris-tui`), whose literals alone changed (the workspace builds with them).

## Left open

- **The tests that fail until the parts after this one land**, each because a yes is now a signature and the command line has no way yet to make one: `cineris-cli`'s system tests of `ext add`, `ext remove` and `permissions trust` (the stub refuses), of the examples and the runs that add an extension, of the interactive application's trust dialog (its yes is unsigned, and refused), and of `serve --stdio`'s conformance; and `cineris-rpc`'s conformance test, whose scenarios answer the trust question with an unsigned yes.
- **`cargo xtask rules`** wants `answered.md` to link this record's folder (`[parts/](parts/)`), which this part does not change; and rows in `docs/dependencies.md` for `argon2`, `ring` and `zeroize`, with `ring` as a direct dependency of `cineris-policy` allowed in the crate graph, both of the policy's own commit.

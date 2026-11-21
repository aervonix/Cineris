//! What a dialog does with each key, in its grace and after it. How the
//! application then acts on that is tested in `app::tests`.

use std::time::Instant;

use cineris_protocol::jsonrpc::RequestId;
use cineris_protocol::methods::{
    ApprovalChoice, ApprovalDetail, ApprovalRequest, CommandDetail, ServerResponse,
};
use cineris_protocol::{CallId, SessionId, TurnId};
use cineris_term::ColorDepth;

use super::*;
use crate::glyphs::Glyphs;
use crate::keys::parse;
use crate::theme::Theme;

fn dialog() -> Dialog {
    let request = ApprovalRequest {
        session_id: SessionId::from("s_1"),
        turn_id: TurnId::from("t_1"),
        call_id: CallId::from("c_1"),
        tool: "shell".to_owned(),
        summary: "Run make".to_owned(),
        detail: ApprovalDetail::Command(CommandDetail {
            command: "make".to_owned(),
            cwd: "/work/app".to_owned(),
            outside_sandbox: false,
        }),
        choices: vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny],
        reasons: Vec::new(),
        adds: Vec::new(),
        hooks: Vec::new(),
        replaced: None,
    };
    Dialog::approval(RequestId::Number(1), request, false)
}

fn key(dialog: &mut Dialog, binding: &str, at: Instant) -> Keyed {
    dialog.key(
        parse(binding).expect("a key"),
        at,
        false,
        false,
        &KeyMap::defaults(),
    )
}

#[test]
fn in_its_grace_typing_passes_and_its_own_other_keys_do_nothing() {
    let now = Instant::now();
    let mut dialog = dialog();
    dialog.drawn(now);
    assert_eq!(key(&mut dialog, "y", now), Keyed::Pass);
    assert_eq!(key(&mut dialog, "esc", now), Keyed::Nothing);
    assert_eq!(key(&mut dialog, "backspace", now), Keyed::Pass);
    assert_eq!(
        key(&mut dialog, "ctrl+c", now),
        Keyed::Global(Action::Interrupt)
    );
}

#[test]
fn ctrl_t_opens_the_rounds_box_from_a_dialog_whenever_it_comes() {
    let now = Instant::now();
    let mut dialog = dialog();
    dialog.drawn(now);
    let look = Look::new(Theme::new(ColorDepth::Mono, None), &Glyphs::UNICODE);
    let _ = dialog.view(&KeyMap::defaults(), 20, 80, &look, now + GRACE);
    for at in [now, now + GRACE] {
        assert_eq!(
            key(&mut dialog, "ctrl+t", at),
            Keyed::Global(Action::ToggleRound)
        );
    }
    assert_eq!(
        key(&mut dialog, "n", now + GRACE),
        Keyed::Answer(ServerResponse::Approval(
            cineris_protocol::methods::ApprovalResponse {
                choice: ApprovalChoice::Deny,
                note: None,
            }
        ))
    );
}

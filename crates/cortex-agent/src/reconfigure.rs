//! A running session's settings changed by a reload of the configuration
//! (`docs/steps/24/answered.md`, "The design, before building", point 1).
//!
//! Two slots, each set through [`SessionHandle`](crate::SessionHandle) and
//! taken by the loop at a boundary, so that nothing in flight changes:
//!
//! - [`Reconfigure`]: what applies at once, taken when the loop prepares
//!   its next request: the accounts' budgets (what was spent is kept), the
//!   turn's budget, the compaction's thresholds and the verification's
//!   rounds. A second change before the request is merged into the first.
//! - [`EpochChange`]: what waits for the next context epoch, taken when a
//!   compaction begins and put in force as its new epoch builds its prompt:
//!   the roles' models, and the context's settings the prompt is built
//!   with (and the compactions after it). A compaction that begins no new
//!   epoch leaves it waiting. A
//!   second one before then replaces the first. The core is told when it is
//!   taken ([`EpochChange::taken`]), so that it can say the keys are in
//!   force.

use cortex_protocol::lease::Budget;
use cortex_protocol::model::ModelRole;

use crate::config::RoleModel;
use crate::ledger::Account;

/// The compaction's thresholds: the `Now` keys of `[context]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Compaction {
    /// `context.auto_compact`.
    pub auto_compact: bool,
    /// `context.background`.
    pub background: bool,
    /// `context.compact_at`.
    pub compact_at: f64,
    /// `context.prepare_at`.
    pub prepare_at: f64,
}

/// What applies to a running session at its next request.
#[derive(Debug, Clone, Default)]
pub struct Reconfigure {
    /// The accounts the session draws on, in place of those it has: each
    /// made from the one before with
    /// [`Account::with_budget`](crate::Account::with_budget), so that what
    /// was spent is kept, or new (a day's limit where there was none).
    pub accounts: Option<Vec<Account>>,
    /// The budget of a turn, the running turn's included.
    pub turn_budget: Option<Budget>,
    /// The compaction's thresholds.
    pub compaction: Option<Compaction>,
    /// The most rounds of verification a turn has.
    pub verify_rounds: Option<u32>,
}

impl Reconfigure {
    /// Whether it changes nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.accounts.is_none()
            && self.turn_budget.is_none()
            && self.compaction.is_none()
            && self.verify_rounds.is_none()
    }

    /// `later` laid over this one: what each sets wins.
    pub(crate) fn merge(&mut self, later: Self) {
        if later.accounts.is_some() {
            self.accounts = later.accounts;
        }
        if later.turn_budget.is_some() {
            self.turn_budget = later.turn_budget;
        }
        if later.compaction.is_some() {
            self.compaction = later.compaction;
        }
        if later.verify_rounds.is_some() {
            self.verify_rounds = later.verify_rounds;
        }
    }
}

/// The context's settings that a new epoch takes: the `NextEpoch` keys of
/// `[context]`.
#[derive(Debug, Clone, PartialEq)]
pub struct EpochContext {
    /// `context.keep_recent_tokens`.
    pub keep_recent_tokens: u64,
    /// The names of instruction files, `context.instruction_names` with the
    /// built-in ones.
    pub instruction_names: Vec<String>,
    /// `context.instructions_cap`, in bytes.
    pub instructions_cap: usize,
}

/// What waits for a running session's next context epoch.
#[derive(Default)]
pub struct EpochChange {
    /// The roles' models, each built with the keys that wait. A role the
    /// person set in the session since is left out by whoever sets this.
    pub models: Vec<(ModelRole, RoleModel)>,
    /// The context's settings.
    pub context: Option<EpochContext>,
    /// Called once, when the loop takes the change: the epoch it was
    /// waiting for has begun.
    pub taken: Option<Box<dyn FnOnce() + Send>>,
}

impl std::fmt::Debug for EpochChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EpochChange")
            .field(
                "models",
                &self
                    .models
                    .iter()
                    .map(|(role, model)| (role.as_str(), model.entry.info.id.as_str()))
                    .collect::<Vec<_>>(),
            )
            .field("context", &self.context)
            .field("taken", &self.taken.is_some())
            .finish()
    }
}

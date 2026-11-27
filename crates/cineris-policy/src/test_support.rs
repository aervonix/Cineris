//! An engine for a test, with sane defaults: behind the `test-support`
//! feature, which only dev-dependencies turn on (`cargo xtask rules`), so
//! that no build of the product has it.
//!
//! [`Parts`] has a field for everything an engine is made from, and a test
//! that wrote them all out by hand had to be changed with every field
//! added to it, in every crate that makes an engine. A test names the
//! workspace and what it is about, and takes the rest from here:
//!
//! ```ignore
//! let engine = TestParts::at(path("/work/app"))
//!     .home(path("/users/dev"))
//!     .level(Level::ReadOnly)
//!     .engine();
//! ```
//!
//! What is not said is the most ordinary session there is (each default is
//! on [`TestParts::at`]): `supervised`, a trusted project, a POSIX shell,
//! an enforcing sandbox, no rules, no approvals, paths read as text, and a
//! fake clock. Anything a setter does not cover is set with
//! [`with`](TestParts::with), or on the [`Parts`] that
//! [`parts`](TestParts::parts) gives.

use std::sync::Arc;

use cineris_base::{AbsPath, FakeClock, SharedClock};
use cineris_protocol::ids::LeaseId;
use cineris_protocol::lease::{Budget, HardLimits, Level, SandboxPolicy};
use cineris_protocol::methods::StoredApproval;

use crate::approvals::ApprovalStore;
use crate::command::ShellSyntax;
use crate::engine::{Engine, Lexical, Locate, Parts};
use crate::lease::default_sandbox;
use crate::level::SandboxState;
use crate::rules::{Protected, SourcedRule};

/// The time the fake clock of [`TestParts`] starts at.
pub const START: &str = "2026-11-13T10:00:00Z";

/// The parts of an engine for a test. See the [module](self).
#[must_use = "a test's parts make nothing until `parts()` or `engine()`"]
pub struct TestParts {
    parts: Parts,
    /// The resolver a test gave; without one, paths are read as text with
    /// the home directory of the parts, whenever it was set.
    locate: Option<Arc<dyn Locate>>,
    /// The resolver the parts start with, which `parts` replaces by the
    /// text one when nothing else took its place: one set with `with` is
    /// kept.
    placeholder: Arc<dyn Locate>,
}

impl std::fmt::Debug for TestParts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestParts")
            .field("parts", &self.parts)
            .field("locate", &self.locate.as_ref().map(|_| "given"))
            .finish()
    }
}

impl TestParts {
    /// The parts of a session in `workspace`, with these defaults:
    ///
    /// | Field | Default |
    /// |---|---|
    /// | `lease_id` | `l_1` |
    /// | `level` | `supervised` |
    /// | `rules`, `approved`, `models` | none |
    /// | `sandbox_policy` | [`default_sandbox`] of the workspace, with nothing more writable or protected |
    /// | `sandbox` | enforcing |
    /// | `budget`, `limits` | their defaults |
    /// | `home` | none |
    /// | `trusted`, `repository_trusted` | `true` |
    /// | `protected` | nothing |
    /// | `shell` | POSIX, whatever the workspace's path style |
    /// | `locate` | [`Lexical`], with the home directory the parts end with |
    /// | `store` | none: "always allow in this project" is not on offer |
    /// | `clock` | a fake clock at [`START`] |
    pub fn at(workspace: AbsPath) -> Self {
        let clock: SharedClock = Arc::new(FakeClock::starting_at(START).unwrap_or_default());
        let placeholder: Arc<dyn Locate> = Arc::new(Lexical::default());
        Self {
            parts: Parts {
                lease_id: LeaseId::new("l_1"),
                level: Level::Supervised,
                rules: Vec::new(),
                approved: Vec::new(),
                sandbox_policy: default_sandbox(&workspace, &[], &[]),
                sandbox: SandboxState::Enforcing,
                models: Vec::new(),
                budget: Budget::default(),
                limits: HardLimits::default(),
                workspace,
                home: None,
                trusted: true,
                repository_trusted: true,
                protected: Protected::none(),
                shell: ShellSyntax::Posix,
                locate: Arc::clone(&placeholder),
                store: None,
                clock,
                own_commands: None,
            },
            locate: None,
            placeholder,
        }
    }

    /// The home directory, which `~` names; also to the default resolver.
    pub fn home(mut self, home: AbsPath) -> Self {
        self.parts.home = Some(home);
        self
    }

    /// The level.
    pub fn level(mut self, level: Level) -> Self {
        self.parts.level = level;
        self
    }

    /// The rules of configuration, flags and learned commands.
    pub fn rules(mut self, rules: Vec<SourcedRule>) -> Self {
        self.parts.rules = rules;
        self
    }

    /// The "always allow in this project" rules, as loaded.
    pub fn approved(mut self, approved: Vec<StoredApproval>) -> Self {
        self.parts.approved = approved;
        self
    }

    /// What a command may touch, in place of the workspace's default.
    pub fn sandbox_policy(mut self, policy: SandboxPolicy) -> Self {
        self.parts.sandbox_policy = policy;
        self
    }

    /// Whether the sandbox confines commands.
    pub fn sandbox(mut self, sandbox: SandboxState) -> Self {
        self.parts.sandbox = sandbox;
        self
    }

    /// Whether the project's allow rules apply.
    pub fn trusted(mut self, trusted: bool) -> Self {
        self.parts.trusted = trusted;
        self
    }

    /// Whether the project is trusted at all, for git's commands that read
    /// the repository's configuration.
    pub fn repository_trusted(mut self, trusted: bool) -> Self {
        self.parts.repository_trusted = trusted;
        self
    }

    /// The harness's own files.
    pub fn protected(mut self, protected: Protected) -> Self {
        self.parts.protected = protected;
        self
    }

    /// The language of the shell tool's commands.
    pub fn shell(mut self, shell: ShellSyntax) -> Self {
        self.parts.shell = shell;
        self
    }

    /// How a path a command names is followed, in place of reading it as
    /// text: a real resolver, for a test on a real file system.
    pub fn locate(mut self, locate: Arc<dyn Locate>) -> Self {
        self.locate = Some(locate);
        self
    }

    /// Where "always allow in this project" is stored.
    pub fn store(mut self, store: ApprovalStore) -> Self {
        self.parts.store = Some(store);
        self
    }

    /// The clock, in place of the fake one at [`START`].
    pub fn clock(mut self, clock: SharedClock) -> Self {
        self.parts.clock = clock;
        self
    }

    /// Changes anything else.
    pub fn with(mut self, change: impl FnOnce(&mut Parts)) -> Self {
        change(&mut self.parts);
        self
    }

    /// The parts.
    pub fn parts(self) -> Parts {
        let Self {
            mut parts,
            locate,
            placeholder,
        } = self;
        if let Some(locate) = locate {
            parts.locate = locate;
        } else if Arc::ptr_eq(&parts.locate, &placeholder) {
            parts.locate = Arc::new(Lexical {
                home: parts.home.clone(),
            });
        }
        parts
    }

    /// The engine.
    ///
    /// # Panics
    ///
    /// When the engine cannot start: a test that means to see that calls
    /// [`Engine::start`] with [`parts`](Self::parts).
    pub fn engine(self) -> Engine {
        match Engine::start(self.parts()) {
            Ok(engine) => engine,
            Err(failure) => panic!("a test's engine did not start: {failure}"),
        }
    }
}

/// The password of [`Person::new`].
pub const PASSWORD: &str = "the person's own password";

/// A person who approves, for a test (the signed approvals step): their key
/// made from a password at a cost small enough for an unoptimised build
/// ([`Person::cost`]), and the propose, sign and commit a yes does, in one
/// call. Tests that check what a session refuses use a second person
/// ([`Person::with_password`]) as the other program's key.
pub struct Person {
    file: crate::signed::KeyFile,
    key: crate::signed::PersonKey,
}

impl std::fmt::Debug for Person {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Person")
            .field("verifier", &self.key.verifier())
            .finish_non_exhaustive()
    }
}

impl Default for Person {
    fn default() -> Self {
        Self::new()
    }
}

impl Person {
    /// The person whose password is [`PASSWORD`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_password(PASSWORD)
    }

    /// A person whose password is `password`, with a salt made from it, so
    /// that the same password makes the same person in every test.
    ///
    /// # Panics
    ///
    /// When `password` has fewer than eight characters.
    #[must_use]
    pub fn with_password(password: &str) -> Self {
        let salt = cineris_base::ContentHash::of(password.as_bytes());
        let mut bytes = [0; crate::signed::Salt::LEN];
        bytes.copy_from_slice(&salt.as_bytes()[..crate::signed::Salt::LEN]);
        let (file, key) = crate::signed::KeyFile::make(
            password.as_bytes(),
            crate::signed::Salt::from_bytes(bytes),
            Self::cost(),
            cineris_protocol::ids::Timestamp::new(START),
        )
        .expect("a test's key is made");
        Self { file, key }
    }

    /// The cost a test's key is made at: 64 KiB and one pass.
    ///
    /// # Panics
    ///
    /// Never: the cost is within the caps.
    #[must_use]
    pub fn cost() -> crate::signed::Cost {
        crate::signed::Cost::new(64, 1, 1).expect("within the caps")
    }

    /// The key file the person's key is made with, as set-up writes it.
    #[must_use]
    pub fn key_file(&self) -> &crate::signed::KeyFile {
        &self.file
    }

    /// What a session that the person opened checks with.
    #[must_use]
    pub fn verifier(&self) -> crate::signed::Verifier {
        self.key.verifier()
    }

    /// The person's signature over `statement`.
    #[must_use]
    pub fn sign(&self, statement: &crate::signed::Statement) -> crate::signed::Signature {
        self.key.sign(statement)
    }

    /// Pins `definitions` in `store` as the person's yes does: proposed,
    /// signed and committed. Gives what is pinned after.
    ///
    /// # Panics
    ///
    /// When the store refuses.
    pub fn pin_all(
        &self,
        store: &crate::trust::PinStore,
        definitions: &[crate::trust::Definition],
    ) -> crate::trust::UserPins {
        let changes: Vec<crate::trust::PinChange> = definitions
            .iter()
            .map(|definition| crate::trust::PinChange::Pin(definition.clone().into()))
            .collect();
        self.change_pins(store, &changes).expect("the pins change")
    }

    /// Pins one definition: [`pin_all`](Self::pin_all).
    ///
    /// # Panics
    ///
    /// When the store refuses.
    pub fn pin(
        &self,
        store: &crate::trust::PinStore,
        definition: &crate::trust::Definition,
    ) -> crate::trust::UserPins {
        self.pin_all(store, std::slice::from_ref(definition))
    }

    /// Takes the pin of this kind and name away, as the person's yes does.
    /// Says whether there was one.
    ///
    /// # Panics
    ///
    /// When the store refuses.
    pub fn unpin(
        &self,
        store: &crate::trust::PinStore,
        kind: &cineris_protocol::methods::TrustResourceKind,
        name: &str,
    ) -> bool {
        let change = crate::trust::PinChange::Unpin {
            kind: kind.clone(),
            name: name.to_owned(),
        };
        let before = store
            .pins(Some(&self.verifier()))
            .expect("the pins are read")
            .revision();
        let after = self.change_pins(store, &[change]).expect("the pins change");
        after.revision() != before
    }

    /// Trusts the project `key` for exactly `definitions`, as the person's
    /// yes to the trust question does: proposed, signed and committed, a
    /// never said of it lifted.
    ///
    /// # Panics
    ///
    /// When the store refuses.
    pub fn trust(
        &self,
        store: &crate::trust::TrustStore,
        key: &cineris_protocol::ids::ProjectKey,
        root: &str,
        definitions: &[crate::trust::Definition],
        git_shown: bool,
    ) -> crate::trust::ProjectTrust {
        let change = crate::trust::TrustChange::Trust {
            root: root.to_owned(),
            approved: definitions.iter().cloned().map(Into::into).collect(),
            git_shown,
            lift_refusal: true,
        };
        self.change_trust(store, key, change)
            .expect("the project is trusted")
    }

    /// Approves `definitions` of the trusted project `key`, as the person's
    /// yes to what is new does.
    ///
    /// # Panics
    ///
    /// When the store refuses.
    pub fn approve(
        &self,
        store: &crate::trust::TrustStore,
        key: &cineris_protocol::ids::ProjectKey,
        definitions: &[crate::trust::Definition],
        git_shown: bool,
    ) -> crate::trust::ProjectTrust {
        let change = crate::trust::TrustChange::Approve {
            approved: definitions.iter().cloned().map(Into::into).collect(),
            git_shown,
        };
        self.change_trust(store, key, change)
            .expect("the definitions are approved")
    }

    /// Makes `change` to the project `key` in `store`, as the person's yes
    /// does: proposed, signed and committed. With nothing to change, the
    /// project's trust as it is.
    ///
    /// # Errors
    ///
    /// What the store refused with.
    pub fn change_trust(
        &self,
        store: &crate::trust::TrustStore,
        key: &cineris_protocol::ids::ProjectKey,
        change: crate::trust::TrustChange,
    ) -> Result<crate::trust::ProjectTrust, crate::trust::TrustError> {
        let verifier = self.verifier();
        match store.propose(key, change, &verifier)? {
            Some(proposal) => store.commit(&proposal, &self.sign(proposal.statement()), &verifier),
            None => store.project(key, Some(&verifier)),
        }
    }

    /// Makes `changes` in `store`, as the person's yes does: proposed,
    /// signed and committed. With nothing to change, the pins as they are.
    ///
    /// # Errors
    ///
    /// What the store refused with.
    pub fn change_pins(
        &self,
        store: &crate::trust::PinStore,
        changes: &[crate::trust::PinChange],
    ) -> Result<crate::trust::UserPins, crate::trust::PinError> {
        let verifier = self.verifier();
        match store.propose(changes, &verifier)? {
            Some(proposal) => store.commit(&proposal, &self.sign(proposal.statement()), &verifier),
            None => store.pins(Some(&verifier)),
        }
    }
}

#[cfg(test)]
mod tests {
    use cineris_base::PathStyle;
    use cineris_protocol::lease::RuleAction;

    use super::*;
    use crate::engine::{Access, Outcome, Request};

    fn path(text: &str) -> AbsPath {
        AbsPath::parse(text, PathStyle::Posix).expect("a path")
    }

    fn read(engine: &Engine, file: &str) -> Outcome {
        engine.decide(&Request {
            tool: "read",
            default: RuleAction::Allow,
            accesses: &[Access::Read { path: path(file) }],
        })
    }

    #[test]
    fn the_defaults_are_an_ordinary_session_in_the_workspace() {
        let parts = TestParts::at(path("/work/app")).parts();
        assert_eq!(parts.level, Level::Supervised);
        assert!(parts.rules.is_empty() && parts.approved.is_empty());
        assert!(parts.trusted && parts.repository_trusted);
        assert_eq!(parts.shell, ShellSyntax::Posix);
        assert_eq!(parts.home, None);
        assert!(parts.store.is_none());
        assert_eq!(parts.sandbox, SandboxState::Enforcing);
        assert_eq!(
            parts.sandbox_policy,
            default_sandbox(&path("/work/app"), &[], &[])
        );
        assert_eq!(Ok(parts.clock.now()), cineris_base::UtcTime::parse(START));
        let engine = TestParts::at(path("/work/app")).engine();
        assert!(
            matches!(
                read(&engine, "/work/app/src/main.rs"),
                Outcome::Allow { .. }
            ),
            "a read in the workspace is allowed"
        );
    }

    #[test]
    fn the_home_directory_is_the_one_the_default_resolver_reads_tilde_as() {
        // Set after the resolver would have been made, it still counts.
        let parts = TestParts::at(path("/work/app"))
            .home(path("/users/dev"))
            .parts();
        assert_eq!(
            parts.locate.locate(&path("/work/app"), "~/notes"),
            Some(path("/users/dev/notes"))
        );
        let given = TestParts::at(path("/work/app"))
            .locate(Arc::new(Lexical::default()))
            .home(path("/users/dev"))
            .parts();
        assert_eq!(
            given.locate.locate(&path("/work/app"), "~/notes"),
            None,
            "a resolver the test gave is the one used"
        );
        // One set through `with` is kept too.
        let through_with = TestParts::at(path("/work/app"))
            .with(|parts| parts.locate = Arc::new(Lexical::default()))
            .home(path("/users/dev"))
            .parts();
        assert_eq!(
            through_with.locate.locate(&path("/work/app"), "~/notes"),
            None,
            "a resolver set through `with` is the one used"
        );
    }

    #[test]
    fn each_setter_sets_its_field() {
        let parts = TestParts::at(path("/work/app"))
            .level(Level::ReadOnly)
            .trusted(false)
            .repository_trusted(false)
            .shell(ShellSyntax::PowerShell)
            .sandbox(SandboxState::NotEnforcing(
                crate::level::NoSandbox::TurnedOff,
            ))
            .with(|parts| parts.lease_id = LeaseId::new("l_9"))
            .parts();
        assert_eq!(parts.level, Level::ReadOnly);
        assert!(!parts.trusted && !parts.repository_trusted);
        assert_eq!(parts.shell, ShellSyntax::PowerShell);
        assert_eq!(
            parts.sandbox,
            SandboxState::NotEnforcing(crate::level::NoSandbox::TurnedOff)
        );
        assert_eq!(parts.lease_id, LeaseId::new("l_9"));
    }
}

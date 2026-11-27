//! What the command line's subcommands ask of the core outside a session
//! (step 13b): the credentials of each provider, the approvals stored for a
//! project, and the project's trust.
//!
//! These are here, and not in the binary, because each one must agree with
//! what a session does: a key is looked for where a session looks for it,
//! a project is the workspace root a session would resolve, and trust is
//! pinned over the very rules a session compares with the pin
//! (`assemble::trust`). A copy of any of them in the binary could drift.
//!
//! Every function here blocks on the file system. The binary calls them from
//! its main thread, outside the core's runtime.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "these run on the binary's main thread, for a subcommand, and never on the core thread"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cineris_base::{ContentHash, PathOptions, PathResolver, Secret};
use cineris_config::{
    CredentialStatus, LoadOptions, Loaded, ProjectTrust as ConfigTrust, StateDir, Withheld,
};
use cineris_policy::{ApprovalStore, Definition, TrustStore};
use cineris_protocol::lease::Rule;
use cineris_protocol::methods::{ProjectEntriesResult, StoredApproval, TrustResourceKind};
use cineris_protocol::project::{EntryChange, NewEntry};
use cineris_protocol::{ApprovalId, ErrorKind, ProjectKey};

use crate::assemble::{held_back_settings, in_git_at, pinned_rules, resolve_dir};
use crate::core::{Core, OpError, error};
use crate::provider::{builtin_names, key_variable};

/// A provider and where its key would come from.
#[derive(Debug)]
pub struct ProviderCredential {
    /// The provider's name.
    pub provider: String,
    /// Where its key comes from, or what went wrong asking.
    pub status: Result<CredentialStatus, String>,
}

/// A workspace as a session would see it: its root and its key.
#[derive(Debug, Clone)]
pub struct Project {
    /// The workspace root, resolved through any link.
    pub root: PathBuf,
    /// The root as a person reads it: `~/work/app`.
    pub shown: String,
    /// The key the stores file it under.
    pub key: ProjectKey,
}

/// Where what the harness knows about a project is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectFiles {
    /// The user's own store for the project, in the state directory.
    pub store: PathBuf,
    /// The repository's shared profile, `.cineris/profile.toml` in the
    /// workspace.
    pub shared: PathBuf,
}

/// Whether the user trusts a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustState {
    /// Not trusted.
    Untrusted,
    /// Trusted, and its rules are the ones the user approved.
    Trusted,
    /// Trusted, and its rules have changed since: what they could loosen is
    /// set aside until the change is approved.
    RulesChanged,
    /// The user said never to trust it: it is not asked about, and nothing
    /// of it that needs trust is used (since step 23).
    Never,
    /// Trusted, its rules the ones the user approved, and a git repository
    /// whose git was not shown when it was trusted (it became one since, or
    /// was trusted under an older store), or that the user kept trusting
    /// without git: git does not read the repository's configuration (since
    /// the follow-ups of step 23).
    GitNotTrusted,
}

impl TrustState {
    /// Whether the project's rules and settings apply: it is trusted, and
    /// its rules are the ones the user approved, whatever was said of git.
    #[must_use]
    pub fn rules_trusted(self) -> bool {
        matches!(self, Self::Trusted | Self::GitNotTrusted)
    }
}

/// What trusting a project pinned.
#[derive(Debug)]
pub struct Trusted {
    /// The project's permission rules, as its files hold them now: every
    /// allow, deny and ask rule, outside any profile and in each profile its
    /// files define, with the profile (`None`: outside any). These are what
    /// a later start compares.
    pub rules: Vec<(Option<String>, Rule)>,
    /// The project's settings that wait for trust, which now apply:
    /// outside any profile and in each profile its files define, with the
    /// profile (`None`: outside any), as the trust question lists them.
    pub settings: Vec<(Option<String>, Withheld)>,
    /// Whether the project was trusted before (its rules are approved
    /// again) or is trusted now for the first time.
    pub was_trusted: bool,
    /// Whether the user had said never to trust it: trusting it lifts that.
    pub was_refused: bool,
    /// Whether the project is a git repository, whose git is trusted now
    /// with the rest (since the follow-ups of step 23).
    pub git: bool,
    /// The names of the project's extensions approved with it, as they
    /// were shown (since step 28).
    pub extensions: Vec<String>,
    /// The names of the project's hooks approved with it, as they were
    /// shown (since step 28).
    pub hooks: Vec<String>,
}

/// One of a project's extensions that `cineris permissions trust` can
/// approve with the project (since step 28): what the trust question shows
/// of it, and the definition approving it pins, of the folder as it was
/// read to be shown.
#[derive(Debug, Clone)]
pub struct ProjectExtension {
    /// Its name.
    pub name: String,
    /// What it may do, in words: the trust question's own text.
    pub detail: String,
    /// Whether it was approved before, at another hash.
    pub changed: bool,
    definition: Definition,
}

impl ProjectExtension {
    /// The hash approving it pins: what `--extension NAME=HASH` names.
    #[must_use]
    pub fn hash(&self) -> ContentHash {
        self.definition.hash
    }
}

/// One of a project's hook commands that `cineris permissions trust` can
/// approve with the project (since step 28): what the trust question shows
/// of it, and the definition approving it pins, kind `hook`, the hash of
/// what is shown.
#[derive(Debug, Clone)]
pub struct ProjectHookCommand {
    /// Its name.
    pub name: String,
    /// What it does, in words: the trust question's own text.
    pub detail: String,
    /// Whether it was approved before, as something else.
    pub changed: bool,
    definition: Definition,
}

impl ProjectHookCommand {
    /// The hash approving it pins: what `--hook NAME=HASH` names.
    #[must_use]
    pub fn hash(&self) -> ContentHash {
        self.definition.hash
    }
}

/// One of a trusted project's extensions or hooks that was approved and is
/// no longer in the project as approved (since the review of step 28,
/// finding 4): `cineris permissions trust` shows it as removed, and a yes
/// takes its approval away, as it was pinned when it was shown.
#[derive(Debug, Clone)]
pub struct ProjectRemoved {
    /// Whether it is an extension or a hook.
    pub kind: TrustResourceKind,
    /// Its name.
    pub name: String,
    /// What the trust question shows of it: that it is removed, what it
    /// gated where that is known, and that a yes takes its approval away.
    pub detail: String,
    definition: Definition,
}

impl Core {
    /// Every provider the core knows (the built-in ones, then the user's
    /// own), with where its key would come from. Nothing is sent anywhere.
    #[must_use]
    pub fn credentials(&self) -> Vec<ProviderCredential> {
        let user = self.inner.user();
        let settings = &user.settings;
        let mut names: Vec<String> = builtin_names().map(str::to_owned).collect();
        for name in settings.providers.keys() {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
        let providers = &self.inner.providers;
        names
            .into_iter()
            .map(|provider| {
                let variable = key_variable(settings, &provider);
                let status =
                    cineris_config::credentials::status(cineris_config::credentials::Lookup {
                        provider: &provider,
                        flag: providers.flags.get(&provider),
                        variable: variable.as_deref(),
                        env: &self.inner.options.env,
                        store: providers.store.as_ref(),
                    })
                    .map_err(|failure| failure.to_string());
                ProviderCredential { provider, status }
            })
            .collect()
    }

    /// The credential store a key is kept in, and where, in words for a
    /// person (since step 25: the first run says where a key it stores
    /// goes, as `auth login` at a terminal does).
    #[must_use]
    pub fn credential_store(&self) -> (cineris_config::StoreKind, String) {
        let store = &self.inner.providers.store;
        (store.kind(), store.describe(&self.inner.options.env))
    }

    /// Whether a session started now with the user's configuration would be
    /// confined (since step 25: the first run offers `autonomous` only
    /// then): the sandbox enforces on this machine and the configuration in
    /// force, managed files and `-c` included, leaves `sandbox.enabled` on.
    /// Since the step 25 fixes; before them it was what the machine can do
    /// alone, so a sandbox turned off by the configuration still offered
    /// `autonomous`, which every session then refused. (`initialize`'s
    /// `sandbox` stays what the machine can do.)
    #[must_use]
    pub fn sandbox_status(&self) -> cineris_protocol::session::SandboxStatus {
        let enabled = self.inner.user().settings.sandbox.enabled;
        crate::sandbox::status_of(self.inner.sandbox.state(enabled).0)
    }

    /// Keeps `key` for `provider` in the credential store.
    ///
    /// # Errors
    ///
    /// Of kind `auth` when the store cannot be opened or written, or the
    /// name is not one a provider can have.
    pub fn store_credential(&self, provider: &str, key: &Secret) -> Result<(), OpError> {
        self.inner
            .providers
            .store
            .set(provider, key)
            .map_err(|failure| error(ErrorKind::Auth, "store_failed", failure.to_string()))
    }

    /// The variable every provider's key is read from, the built-in ones'
    /// and each configured one's, and those a provider's header reads: the
    /// list a shell command is given none of. The first run gives an
    /// imported MCP server's header none of them (since the step 25 fixes,
    /// the review: `anthropic` and `api-key` made `ANTHROPIC_API_KEY`).
    #[must_use]
    pub fn provider_key_variables(&self) -> Vec<String> {
        crate::provider::key_variables(&self.inner.user().settings)
    }

    /// The model the first run writes as `model.main` when `provider` is
    /// chosen: the one the model catalog marks as the provider's default,
    /// a user's own `[models.*]` entry marked so in place of the built-in
    /// one (since the review of the fixes of step 25's third pull request;
    /// before it, the first the catalog listed, which for Anthropic was its
    /// dearest). `None` when none is marked, as for a local server, whose
    /// models the catalog does not know.
    #[must_use]
    pub fn default_model(&self, provider: &str) -> Option<String> {
        self.inner
            .providers
            .catalog
            .models()
            .iter()
            .find(|entry| entry.default && entry.info.provider.as_str() == provider)
            .map(|entry| entry.info.id.as_str().to_owned())
    }

    /// Whether more than one provider has a model of the id `model` in the
    /// catalog, the user's own entries included: the provider must then be
    /// named beside it (`model.roles.<role>.provider`), and the first run
    /// writes it only then (since the review of the fixes of step 25's third
    /// pull request).
    #[must_use]
    pub fn model_shared(&self, model: &str) -> bool {
        let mut providers = self
            .inner
            .providers
            .catalog
            .models()
            .iter()
            .filter(|entry| entry.info.id.as_str() == model)
            .map(|entry| &entry.info.provider);
        let first = providers.next();
        providers.any(|other| Some(other) != first)
    }

    /// Removes the stored key of `provider`. Says whether there was one.
    ///
    /// # Errors
    ///
    /// Of kind `auth` when the store cannot be opened or written.
    pub fn remove_credential(&self, provider: &str) -> Result<bool, OpError> {
        self.inner
            .providers
            .store
            .remove(provider)
            .map_err(|failure| error(ErrorKind::Auth, "store_failed", failure.to_string()))
    }

    /// The project a session started in `directory` would be in.
    ///
    /// # Errors
    ///
    /// Of kind `usage` when the directory cannot be a workspace.
    pub fn project(&self, directory: &Path) -> Result<Project, OpError> {
        let resolver = self.resolver(directory)?;
        let root = resolver.root();
        Ok(Project {
            root: root.to_fs_path(),
            shown: self.inner.options.env.display_path(&root.to_fs_path()),
            key: root.project_key(),
        })
    }

    /// Every entry of a project's profile and memory, with the standing
    /// each has for this user, and the files that could not be read
    /// (`cineris project list`). An entry past its expiry is marked first.
    ///
    /// # Errors
    ///
    /// Of kind `usage` when there is no state directory.
    pub fn project_entries(&self, project: &Project) -> Result<ProjectEntriesResult, OpError> {
        let (knowledge, _) = crate::knowledge::at(self, &project.root)?;
        Ok(crate::knowledge::listing(&knowledge))
    }

    /// Changes a project's entries as its user: `changes` to entries
    /// there are, and `add`, entries the user writes. It is what
    /// `project/update` does for a `user` client, with the same checks,
    /// all or nothing. Answers with the entries that changed.
    ///
    /// A session of the project in another process takes the change at
    /// its next turn's start, and a command turned off before its next
    /// decision.
    ///
    /// # Errors
    ///
    /// Of kind `usage` with the operation's codes: an entry that is not
    /// there, a change that cannot be made to it, a text that holds what
    /// looks like a credential, a store that is full or cannot be read as
    /// it is.
    pub fn change_project_entries(
        &self,
        project: &Project,
        changes: &[EntryChange],
        add: &[NewEntry],
    ) -> Result<ProjectEntriesResult, OpError> {
        let (knowledge, _) = crate::knowledge::at(self, &project.root)?;
        let updated =
            crate::knowledge::apply(&knowledge, &self.inner.providers.filter, None, changes, add)?;
        Ok(ProjectEntriesResult {
            entries: updated
                .changed
                .iter()
                .map(cineris_knowledge::Entry::to_wire)
                .collect(),
            problems: updated.snapshot.problems().to_vec(),
        })
    }

    /// Where a project's knowledge is kept (`cineris project path`).
    ///
    /// # Errors
    ///
    /// Of kind `usage` when there is no state directory.
    pub fn project_files(&self, project: &Project) -> Result<ProjectFiles, OpError> {
        let (knowledge, _) = crate::knowledge::at(self, &project.root)?;
        Ok(ProjectFiles {
            store: knowledge.store().path().to_path_buf(),
            shared: knowledge.shared_file(),
        })
    }

    /// The approvals stored for a project, oldest first.
    ///
    /// # Errors
    ///
    /// When the state directory is unknown or the file cannot be read.
    pub fn stored_approvals(&self, project: &Project) -> Result<Vec<StoredApproval>, OpError> {
        self.approval_store(project)?
            .load()
            .map_err(|failure| error(ErrorKind::Internal, "store_failed", failure.to_string()))
    }

    /// Removes stored approvals of a project. Says how many there were.
    ///
    /// # Errors
    ///
    /// When the state directory is unknown or the file cannot be written.
    pub fn revoke_approvals(
        &self,
        project: &Project,
        ids: &[ApprovalId],
    ) -> Result<usize, OpError> {
        self.approval_store(project)?
            .remove(ids)
            .map_err(|failure| error(ErrorKind::Internal, "store_failed", failure.to_string()))
    }

    /// Whether the user trusts a project, judged as a session judges it.
    ///
    /// # Errors
    ///
    /// When the trust store or the project's configuration cannot be read.
    pub fn project_trust(&self, project: &Project) -> Result<TrustState, OpError> {
        let stored = self
            .trust_store()?
            .project(&project.key)
            .map_err(|failure| error(ErrorKind::Internal, "store_failed", failure.to_string()))?;
        if stored.is_refused() {
            return Ok(TrustState::Never);
        }
        if !stored.is_trusted() {
            return Ok(TrustState::Untrusted);
        }
        let rules = self.pinned(project)?;
        Ok(if !stored.project_rules_approved(&rules) {
            TrustState::RulesChanged
        } else if !stored.git_trusted() && in_git_at(&project.root) {
            TrustState::GitNotTrusted
        } else {
            TrustState::Trusted
        })
    }

    /// Trusts a project, or approves the change of its rules: pins every
    /// permission rule its files hold now (allow, deny and ask, which is
    /// what a later start compares) and keeps whatever else was pinned. In
    /// a git repository git is trusted with it, and said to be
    /// ([`Trusted::git`]), as the trust question lists it.
    ///
    /// # Errors
    ///
    /// When the project's configuration is invalid, or the trust store
    /// cannot be read or written.
    pub fn trust_project(&self, project: &Project) -> Result<Trusted, OpError> {
        self.trust_project_with(project, &[], &[])
    }

    /// What trusting the project would turn the sandbox off with, in the
    /// trust question's words: each write root of its own configuration, in
    /// each of its profiles too, that holds the harness's files, where the
    /// sandbox would otherwise enforce (`assemble::harness_write_warnings`).
    /// Said of a project already trusted too: what it held back is judged
    /// against the configuration with no trust.
    ///
    /// # Errors
    ///
    /// When the workspace cannot be resolved, or its configuration loaded.
    pub fn project_sandbox_warnings(&self, project: &Project) -> Result<Vec<String>, OpError> {
        let options = &self.inner.options;
        let mut paths = PathOptions::new();
        if let Some(home) = options.env.home().as_deref().and_then(resolve_dir) {
            paths = paths.home(home);
        }
        let resolver = PathResolver::new(&project.root, paths)
            .map_err(|failure| error(ErrorKind::Usage, "invalid_workspace", failure.to_string()))?;
        let held = self.load(project, ConfigTrust::Untrusted)?;
        let trusted = self.load_trusted(project)?;
        crate::assemble::harness_write_warnings(
            &self.inner,
            resolver.root(),
            self.state_dir().as_ref(),
            (&held, &trusted),
            &|profile| {
                Ok((
                    self.load_profile(project, profile, ConfigTrust::Untrusted)?,
                    self.load_profile(project, profile, ConfigTrust::Trusted)?,
                ))
            },
        )
    }

    /// The project's extensions that trusting it could approve: those not
    /// approved as they are now, each as the trust question lists it (one
    /// whose name a folder of the user's own extensions takes is not
    /// listed: it is not used whatever is approved). Nothing is approved.
    ///
    /// # Errors
    ///
    /// When the workspace cannot be resolved, or the trust store cannot be
    /// read.
    pub fn project_extensions(&self, project: &Project) -> Result<Vec<ProjectExtension>, OpError> {
        let options = &self.inner.options;
        let mut paths = PathOptions::new();
        if let Some(home) = options.env.home().as_deref().and_then(resolve_dir) {
            paths = paths.home(home);
        }
        let resolver = PathResolver::new(&project.root, paths)
            .map_err(|failure| error(ErrorKind::Usage, "invalid_workspace", failure.to_string()))?;
        let stored = self
            .trust_store()?
            .project(&project.key)
            .map_err(|failure| error(ErrorKind::Internal, "store_failed", failure.to_string()))?;
        let (found, _) = crate::ext::found::in_project(&resolver, &options.clock);
        if found.found.is_empty() {
            return Ok(Vec::new());
        }
        // Whether its programs would be confined, which what it is shown
        // as says.
        let loaded = self.load_trusted(project)?;
        let confined = crate::assemble::programs_confined(
            &self.inner,
            &loaded,
            resolver.root(),
            self.state_dir().as_ref(),
        );
        let dirs = options
            .dirs
            .clone()
            .unwrap_or_else(|| cineris_config::Dirs::from_env(&options.env));
        let users = crate::ext::found::UserNames::listed(&resolver, dirs.config.as_deref());
        let asked = crate::ext::found::asked(&found, &users, &stored, confined);
        Ok(asked
            .resources
            .into_iter()
            .zip(asked.pins)
            .map(|(resource, definition)| ProjectExtension {
                name: resource.name,
                detail: resource.detail,
                changed: resource.changed,
                definition,
            })
            .collect())
    }

    /// The project's hook commands that trusting it could approve: those
    /// not approved as they are now, each as the trust question lists it.
    /// One whose name is one of the user's own hooks' is not listed: it
    /// never runs. Nothing is approved.
    ///
    /// # Errors
    ///
    /// When the workspace cannot be resolved, its configuration loaded, or
    /// the trust store read.
    pub fn project_hook_commands(
        &self,
        project: &Project,
    ) -> Result<Vec<ProjectHookCommand>, OpError> {
        let loaded = self.load_trusted(project)?;
        let configured = crate::ext::hooks::Configured::of(&loaded);
        if configured.project.is_empty() {
            return Ok(Vec::new());
        }
        let stored = self
            .trust_store()?
            .project(&project.key)
            .map_err(|failure| error(ErrorKind::Internal, "store_failed", failure.to_string()))?;
        let root = self.resolver(&project.root)?.root().clone();
        // Whether a hook's program would be confined, which what it is
        // shown as says.
        let confined = crate::assemble::programs_confined(
            &self.inner,
            &loaded,
            &root,
            self.state_dir().as_ref(),
        );
        let asked = crate::ext::hooks::asked(&configured, &stored, confined);
        Ok(asked
            .resources
            .into_iter()
            .zip(asked.pins)
            .map(|(resource, definition)| ProjectHookCommand {
                name: resource.name,
                detail: resource.detail,
                changed: resource.changed,
                definition,
            })
            .collect())
    }

    /// [`trust_project`](Self::trust_project), approving `extensions` and
    /// `hooks` with it (from [`project_extensions`](Self::project_extensions)
    /// and [`project_hook_commands`](Self::project_hook_commands), which the
    /// person was shown): each pinned as it was read to be shown.
    ///
    /// # Errors
    ///
    /// As [`trust_project`](Self::trust_project).
    pub fn trust_project_with(
        &self,
        project: &Project,
        extensions: &[ProjectExtension],
        hooks: &[ProjectHookCommand],
    ) -> Result<Trusted, OpError> {
        let rules = self.pinned(project)?;
        // What an untrusted load holds back is what trust now turns on.
        let settings =
            held_back_settings(&self.load(project, ConfigTrust::Untrusted)?, &|profile| {
                self.load_profile(project, profile, ConfigTrust::Untrusted)
            })?;
        let store = self.trust_store()?;
        let failed = |failure: cineris_policy::TrustError| {
            error(ErrorKind::Internal, "store_failed", failure.to_string())
        };
        let stored = store.project(&project.key).map_err(failed)?;
        let (was_trusted, was_refused) = (stored.is_trusted(), stored.is_refused());
        let mut pin = vec![Definition::of_project_rules(&rules)];
        pin.extend(
            extensions
                .iter()
                .map(|extension| extension.definition.clone()),
        );
        pin.extend(hooks.iter().map(|hook| hook.definition.clone()));
        let git = in_git_at(&project.root);
        if was_trusted {
            store.approve(&project.key, &pin, git).map_err(failed)?;
        } else {
            store
                .trust(&project.key, &project.shown, &pin, git)
                .map_err(failed)?;
        }
        Ok(Trusted {
            rules: rules
                .each()
                .map(|(profile, rule)| (profile.map(str::to_owned), rule.clone()))
                .collect(),
            settings,
            was_trusted,
            was_refused,
            git,
            extensions: extensions
                .iter()
                .map(|extension| extension.name.clone())
                .collect(),
            hooks: hooks.iter().map(|hook| hook.name.clone()).collect(),
        })
    }

    /// What of a trusted project was approved and is no longer in it as
    /// approved, each as the trust question lists it as removed: its
    /// extensions gone from its folder, and its hooks with no entry that
    /// runs as approved (taken out, turned off, or no longer compiling).
    /// Nothing is taken away.
    ///
    /// # Errors
    ///
    /// When the workspace cannot be resolved, its configuration loaded, or
    /// the trust store read.
    pub fn project_removed(&self, project: &Project) -> Result<Vec<ProjectRemoved>, OpError> {
        let stored = self
            .trust_store()?
            .project(&project.key)
            .map_err(|failure| error(ErrorKind::Internal, "store_failed", failure.to_string()))?;
        let mut removed = Vec::new();
        if !stored
            .pinned_of(&TrustResourceKind::Extension)
            .is_empty()
        {
            let options = &self.inner.options;
            let resolver = self.resolver(&project.root)?;
            let (found, _) = crate::ext::found::in_project(&resolver, &options.clock);
            let dirs = options
                .dirs
                .clone()
                .unwrap_or_else(|| cineris_config::Dirs::from_env(&options.env));
            let users = crate::ext::found::UserNames::listed(&resolver, dirs.config.as_deref());
            let cache = self.state_dir().map(|state| {
                cineris_ext::cache::RegistrationCache::new(state.registration_cache_file())
                    .with_clock(Arc::clone(&options.clock))
                    .load()
                    .cache
            });
            let asked = crate::ext::found::removed(&found, &users, &stored, cache.as_ref());
            removed.extend(asked.resources.into_iter().zip(asked.removed));
        }
        if !stored.pinned_of(&TrustResourceKind::Hook).is_empty() {
            let loaded = self.load_trusted(project)?;
            let configured = crate::ext::hooks::Configured::of(&loaded);
            let asked = crate::ext::hooks::asked(&configured, &stored, false);
            removed.extend(asked.resources.into_iter().skip(asked.pins.len()).zip(asked.removed));
        }
        Ok(removed
            .into_iter()
            .map(|(resource, definition)| ProjectRemoved {
                kind: resource.kind,
                name: resource.name,
                detail: resource.detail,
                definition,
            })
            .collect())
    }

    /// [`trust_project_with`](Self::trust_project_with), and then takes away
    /// the approval of each of `removed` (from
    /// [`project_removed`](Self::project_removed), which the person was
    /// shown), as it was pinned when it was shown. Says how many approvals
    /// went with the rest.
    ///
    /// # Errors
    ///
    /// As [`trust_project_with`](Self::trust_project_with), and when the
    /// store cannot be written.
    pub fn trust_project_dropping(
        &self,
        project: &Project,
        extensions: &[ProjectExtension],
        hooks: &[ProjectHookCommand],
        removed: &[ProjectRemoved],
    ) -> Result<(Trusted, usize), OpError> {
        let trusted = self.trust_project_with(project, extensions, hooks)?;
        if removed.is_empty() || !trusted.was_trusted {
            return Ok((trusted, 0));
        }
        let pins: Vec<Definition> = removed.iter().map(|one| one.definition.clone()).collect();
        let dropped = self
            .trust_store()?
            .drop_pins(&project.key, &pins)
            .map_err(|failure| error(ErrorKind::Internal, "store_failed", failure.to_string()))?;
        Ok((trusted, dropped))
    }

    /// Takes trust away from a project. Says whether it was trusted.
    ///
    /// # Errors
    ///
    /// When the trust store cannot be read or written.
    pub fn untrust_project(&self, project: &Project) -> Result<bool, OpError> {
        self.trust_store()?
            .revoke(&project.key)
            .map_err(|failure| error(ErrorKind::Internal, "store_failed", failure.to_string()))
    }

    /// Forgets what the user said of a project, trust and never alike, so
    /// that the next session there asks again. Says whether there was
    /// anything to forget.
    ///
    /// # Errors
    ///
    /// When the trust store cannot be read or written.
    pub fn forget_project(&self, project: &Project) -> Result<bool, OpError> {
        self.trust_store()?
            .forget(&project.key)
            .map_err(|failure| error(ErrorKind::Internal, "store_failed", failure.to_string()))
    }

    /// Every project the user trusts, by the root it was trusted at.
    ///
    /// # Errors
    ///
    /// When the trust store cannot be read.
    pub fn trusted_projects(&self) -> Result<Vec<String>, OpError> {
        Ok(self
            .trust_store()?
            .list()
            .map_err(|failure| error(ErrorKind::Internal, "store_failed", failure.to_string()))?
            .into_iter()
            .map(|(_, root)| root)
            .collect())
    }

    /// The configuration a session in `project` would have: the project's
    /// layers read with its trust as a session judges it. With `strict`, an
    /// unknown key is an error (`cineris config check`).
    ///
    /// # Errors
    ///
    /// Of kind `config`, with every problem of every layer, when it is not
    /// valid.
    pub fn configuration(&self, project: &Project, strict: bool) -> Result<Loaded, OpError> {
        let trusted =
            self.inner.options.trust_project || self.project_trust(project)?.rules_trusted();
        let trust = if trusted {
            ConfigTrust::Trusted
        } else {
            ConfigTrust::Untrusted
        };
        self.load_with(project, trust, strict)
    }

    pub(crate) fn resolver(&self, directory: &Path) -> Result<PathResolver, OpError> {
        let mut options = PathOptions::new();
        if let Some(home) = self
            .inner
            .options
            .env
            .home()
            .as_deref()
            .and_then(resolve_dir)
        {
            options = options.home(home);
        }
        PathResolver::new(directory, options).map_err(|failure| {
            error(
                ErrorKind::Usage,
                "invalid_workspace",
                format!("`{}` cannot be a workspace: {failure}", directory.display()),
            )
        })
    }

    pub(crate) fn state(&self) -> Result<StateDir, OpError> {
        self.state_dir().ok_or_else(|| {
            error(
                ErrorKind::Config,
                "no_state_dir",
                "No state directory could be worked out: the home directory is not known.",
            )
        })
    }

    pub(crate) fn trust_store(&self) -> Result<TrustStore, OpError> {
        Ok(TrustStore::new(
            self.state()?.trust_file(),
            Arc::clone(&self.inner.options.clock),
        ))
    }

    fn approval_store(&self, project: &Project) -> Result<ApprovalStore, OpError> {
        let file = self.state()?.approvals_file(&project.key).ok_or_else(|| {
            error(
                ErrorKind::Internal,
                "no_approvals_file",
                "The project has no place for stored approvals.",
            )
        })?;
        Ok(ApprovalStore::new(
            file,
            Arc::clone(&self.inner.options.clock),
        ))
    }

    pub(crate) fn load(&self, project: &Project, trust: ConfigTrust) -> Result<Loaded, OpError> {
        self.load_with(project, trust, false)
    }

    fn load_with(
        &self,
        project: &Project,
        trust: ConfigTrust,
        strict: bool,
    ) -> Result<Loaded, OpError> {
        let options = &self.inner.options;
        let mut load = LoadOptions::new(options.env.clone());
        if let Some(dirs) = &options.dirs {
            load.dirs = dirs.clone();
        }
        load.workspace = Some(project.root.clone());
        load.profile.clone_from(&options.profile);
        load.overrides.clone_from(&options.overrides);
        load.trust = trust;
        load.strict = strict;
        cineris_config::load(&load).map_err(|failure| {
            error(
                ErrorKind::Config,
                "invalid_configuration",
                failure.render(&options.env),
            )
        })
    }

    fn load_trusted(&self, project: &Project) -> Result<Loaded, OpError> {
        self.load(project, ConfigTrust::Trusted)
    }

    /// The project's rules as trust pins them, in every profile its files
    /// define, as a session's start compares them.
    fn pinned(&self, project: &Project) -> Result<cineris_policy::ProjectRules, OpError> {
        let loaded = self.load_trusted(project)?;
        pinned_rules(&loaded, &|profile| {
            self.load_profile(project, profile, ConfigTrust::Trusted)
        })
    }

    /// The project's configuration with `profile` selected.
    fn load_profile(
        &self,
        project: &Project,
        profile: &str,
        trust: ConfigTrust,
    ) -> Result<Loaded, OpError> {
        let options = &self.inner.options;
        let mut load = LoadOptions::new(options.env.clone());
        if let Some(dirs) = &options.dirs {
            load.dirs = dirs.clone();
        }
        load.workspace = Some(project.root.clone());
        load.profile = Some(profile.to_owned());
        load.overrides.clone_from(&options.overrides);
        load.trust = trust;
        cineris_config::load(&load).map_err(|failure| {
            error(
                ErrorKind::Config,
                "invalid_configuration",
                failure.render(&options.env),
            )
        })
    }
}

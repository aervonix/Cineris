//! Where an extension comes from: the user's own folder, or a project's.
//!
//! It decides which approval an extension needs and where its pin is kept,
//! and it keys what an extension keeps (its `cineris.state`), so that a
//! project's `tracker` never reads or writes the user's `tracker`'s values
//! (`docs/steps/28/answered.md`, points 4 and 5).

/// Where an extension was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Source {
    /// The user's own, in `<config dir>/extensions/`: pinned in the state
    /// directory's `pins.toml`, approved with `cineris ext add`.
    User,
    /// The project's, in the workspace's `.cineris/extensions/`, found only
    /// once the project is trusted: pinned in the project's entry of
    /// `trust.toml`, approved through the trust question.
    Project,
}

impl Source {
    /// The word for it, in a file's name and in what the user is told:
    /// `user` or `project`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
        }
    }
}

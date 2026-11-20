//! The seven events a hook may run on (`docs/architecture/12-extensions.md`,
//! "Tier 3: hooks"). The set is small on purpose: every event is a
//! compatibility promise.

/// An event of the agent's lifecycle that hooks may run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Event {
    /// A session opens.
    SessionStart,
    /// The user sends a prompt, before it reaches the model.
    PromptSubmit,
    /// A tool call has been planned and passed policy, before it runs.
    ToolBefore,
    /// A tool call finished.
    ToolAfter,
    /// A turn ended.
    TurnEnd,
    /// Before a summary is requested.
    CompactionBefore,
    /// The session closes.
    SessionEnd,
}

impl Event {
    /// Every event, in the order of a session's life.
    pub const ALL: [Self; 7] = [
        Self::SessionStart,
        Self::PromptSubmit,
        Self::ToolBefore,
        Self::ToolAfter,
        Self::TurnEnd,
        Self::CompactionBefore,
        Self::SessionEnd,
    ];

    /// The event's name, as configuration and scripts write it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionStart => "session.start",
            Self::PromptSubmit => "prompt.submit",
            Self::ToolBefore => "tool.before",
            Self::ToolAfter => "tool.after",
            Self::TurnEnd => "turn.end",
            Self::CompactionBefore => "compaction.before",
            Self::SessionEnd => "session.end",
        }
    }

    /// The event of a name, or `None`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|event| event.as_str() == name)
    }

    /// Whether a hook's answer can stop what happens: a failed or late hook
    /// on such an event counts as `deny`.
    #[must_use]
    pub const fn gates(self) -> bool {
        matches!(self, Self::PromptSubmit | Self::ToolBefore)
    }

    /// What a hook on the event is given to see, in words for the person
    /// who approves it.
    #[must_use]
    pub const fn sees(self) -> &'static str {
        match self {
            Self::SessionStart => "when a session opens, and its workspace",
            Self::PromptSubmit => "every prompt you send, before the model does",
            Self::ToolBefore => {
                "every tool call's arguments, before it runs, and may refuse or change it"
            }
            Self::ToolAfter => "every tool call's arguments and result",
            Self::TurnEnd => "when each turn ends, and how",
            Self::CompactionBefore => "when the conversation is summarised",
            Self::SessionEnd => "when a session closes",
        }
    }
}

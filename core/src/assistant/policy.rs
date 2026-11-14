//! Which tool calls the assistant may run on its own. The one place that maps
//! a tool to an action category (`docs/architecture/ai-permissions.md`
//! section 2); everything it does not know is asked, never run.
use crate::{
    DraftOrigin,
    settings::{AssistantMode, AssistantSendMode, AssistantSettings},
    tools::{AccountScope, Grant},
};

/// Whose draft a draft tool acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DraftSource {
    /// Created by the assistant itself.
    Assistant,
    /// Written by the user or created over MCP.
    Other,
    /// No such draft in the grant's scope; the tool reports `notFound` and
    /// changes nothing, so there is nothing to ask about.
    Missing,
    /// The draft could not be looked up.
    Unknown,
}

impl DraftSource {
    pub(crate) fn of(origin: DraftOrigin) -> Self {
        match origin {
            DraftOrigin::Ai => Self::Assistant,
            DraftOrigin::User | DraftOrigin::Mcp => Self::Other,
        }
    }
}

/// What to do with one tool call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    Run,
    /// The user must agree first. Until the confirmation flow exists, the
    /// loop treats this as declined.
    Ask,
}

const READ_TOOLS: &[&str] = &[
    "list_accounts",
    "search_emails",
    "read_email",
    "list_folders",
    "list_drafts",
    "read_draft",
    "list_outbox",
    "get_outbox",
];
const ORGANIZE_TOOLS: &[&str] = &[
    "set_email_flags",
    "archive_emails",
    "unarchive_emails",
    "refresh_sync",
];
const NEW_DRAFT_TOOLS: &[&str] = &["create_draft", "create_reply_draft"];
const DRAFT_CHANGE_TOOLS: &[&str] = &[
    "update_draft",
    "delete_draft",
    "add_draft_attachment",
    "remove_draft_attachment",
];
const SEND_TOOLS: &[&str] = &["send_email", "cancel_outbox"];

/// Whether the tool changes an existing draft, so the loop must look the
/// draft up before deciding.
pub(crate) fn needs_draft(tool: &str) -> bool {
    DRAFT_CHANGE_TOOLS.contains(&tool)
}

fn mode(mode: AssistantMode) -> Decision {
    match mode {
        AssistantMode::Direct => Decision::Run,
        AssistantMode::Ask => Decision::Ask,
    }
}

/// `source` only matters for tools that change an existing draft.
pub(crate) fn classify(tool: &str, settings: &AssistantSettings, source: DraftSource) -> Decision {
    if READ_TOOLS.contains(&tool) {
        Decision::Run
    } else if ORGANIZE_TOOLS.contains(&tool) {
        mode(settings.organize)
    } else if NEW_DRAFT_TOOLS.contains(&tool) {
        mode(settings.ai_drafts)
    } else if DRAFT_CHANGE_TOOLS.contains(&tool) {
        match source {
            DraftSource::Assistant => mode(settings.ai_drafts),
            DraftSource::Missing => Decision::Run,
            DraftSource::Other | DraftSource::Unknown => Decision::Ask,
        }
    } else if SEND_TOOLS.contains(&tool) {
        match settings.send {
            AssistantSendMode::Direct => Decision::Run,
            AssistantSendMode::Confirm => Decision::Ask,
        }
    } else {
        Decision::Ask
    }
}

/// The grant for one call: every action the local MCP grant holds, over the
/// accounts the user left available. `all_accounts` are the ids that exist
/// now; nothing excluded keeps the open scope.
pub(crate) fn grant(settings: &AssistantSettings, all_accounts: &[String]) -> Grant {
    let accounts = if settings.excluded_accounts.is_empty() {
        AccountScope::All
    } else {
        AccountScope::Only(
            all_accounts
                .iter()
                .filter(|id| !settings.excluded_accounts.contains(id))
                .cloned()
                .collect(),
        )
    };
    Grant::new(accounts, Grant::mcp_default().actions.iter().copied())
}

/// Every action a grant holds, for tests.
#[cfg(test)]
pub(crate) fn actions(grant: &Grant) -> Vec<crate::tools::Action> {
    grant.actions.iter().copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(
        organize: AssistantMode,
        ai_drafts: AssistantMode,
        send: AssistantSendMode,
    ) -> AssistantSettings {
        AssistantSettings {
            organize,
            ai_drafts,
            send,
            ..AssistantSettings::default()
        }
    }

    fn all(decision: Decision, s: &AssistantSettings, tools: &[&str], source: DraftSource) {
        for tool in tools {
            assert_eq!(classify(tool, s, source), decision, "{tool} {source:?}");
        }
    }

    #[test]
    fn reads_always_run() {
        for organize in [AssistantMode::Direct, AssistantMode::Ask] {
            let s = settings(organize, AssistantMode::Ask, AssistantSendMode::Confirm);
            all(Decision::Run, &s, READ_TOOLS, DraftSource::Unknown);
        }
    }

    #[test]
    fn organize_follows_its_setting() {
        let direct = settings(
            AssistantMode::Direct,
            AssistantMode::Ask,
            AssistantSendMode::Confirm,
        );
        let ask = settings(
            AssistantMode::Ask,
            AssistantMode::Direct,
            AssistantSendMode::Direct,
        );
        all(Decision::Run, &direct, ORGANIZE_TOOLS, DraftSource::Missing);
        all(Decision::Ask, &ask, ORGANIZE_TOOLS, DraftSource::Missing);
    }

    #[test]
    fn new_drafts_and_the_assistants_own_drafts_follow_ai_drafts() {
        let direct = settings(
            AssistantMode::Ask,
            AssistantMode::Direct,
            AssistantSendMode::Confirm,
        );
        let ask = settings(
            AssistantMode::Direct,
            AssistantMode::Ask,
            AssistantSendMode::Direct,
        );
        all(Decision::Run, &direct, NEW_DRAFT_TOOLS, DraftSource::Other);
        all(Decision::Ask, &ask, NEW_DRAFT_TOOLS, DraftSource::Other);
        all(
            Decision::Run,
            &direct,
            DRAFT_CHANGE_TOOLS,
            DraftSource::Assistant,
        );
        all(
            Decision::Ask,
            &ask,
            DRAFT_CHANGE_TOOLS,
            DraftSource::Assistant,
        );
    }

    #[test]
    fn other_peoples_drafts_are_always_asked_even_when_everything_is_direct() {
        let s = settings(
            AssistantMode::Direct,
            AssistantMode::Direct,
            AssistantSendMode::Direct,
        );
        all(Decision::Ask, &s, DRAFT_CHANGE_TOOLS, DraftSource::Other);
        all(Decision::Ask, &s, DRAFT_CHANGE_TOOLS, DraftSource::Unknown);
        // Nothing to protect: the tool answers notFound.
        all(Decision::Run, &s, DRAFT_CHANGE_TOOLS, DraftSource::Missing);
        for tool in DRAFT_CHANGE_TOOLS {
            assert!(needs_draft(tool));
        }
        assert!(!needs_draft("create_draft") && !needs_draft("send_email"));
    }

    #[test]
    fn sending_is_confirmed_unless_the_user_chose_direct() {
        let confirm = settings(
            AssistantMode::Direct,
            AssistantMode::Direct,
            AssistantSendMode::Confirm,
        );
        let direct = settings(
            AssistantMode::Ask,
            AssistantMode::Ask,
            AssistantSendMode::Direct,
        );
        all(Decision::Ask, &confirm, SEND_TOOLS, DraftSource::Missing);
        all(Decision::Run, &direct, SEND_TOOLS, DraftSource::Missing);
    }

    #[test]
    fn unknown_tools_are_asked() {
        let s = settings(
            AssistantMode::Direct,
            AssistantMode::Direct,
            AssistantSendMode::Direct,
        );
        all(
            Decision::Ask,
            &s,
            &["delete_email", "", "show_email_in_app"],
            DraftSource::Missing,
        );
    }

    #[test]
    fn every_tool_the_agent_can_see_is_classified() {
        let known: Vec<&str> = [
            READ_TOOLS,
            ORGANIZE_TOOLS,
            NEW_DRAFT_TOOLS,
            DRAFT_CHANGE_TOOLS,
            SEND_TOOLS,
        ]
        .concat();
        // Mirrors the registry: the agent's tool set is exactly these.
        let mut sorted = known.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), known.len(), "a tool is in two categories");
    }

    #[test]
    fn grant_covers_every_mcp_action_and_hides_excluded_accounts() {
        let open = grant(&AssistantSettings::default(), &["a".into(), "b".into()]);
        assert_eq!(open.accounts, AccountScope::All);
        assert_eq!(actions(&open), actions(&Grant::mcp_default()));
        let narrowed = grant(
            &AssistantSettings {
                excluded_accounts: vec!["b".into()],
                ..AssistantSettings::default()
            },
            &["a".into(), "b".into(), "c".into()],
        );
        assert_eq!(
            narrowed.accounts,
            AccountScope::Only(vec!["a".into(), "c".into()])
        );
    }
}

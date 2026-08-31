use crate::{
    app::composer::user_invocations::ComposerSuggestion,
    protocol::{SlashCommand, SlashCommandSource},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) enum BuiltinSlashCommand {
    Settings,
    Model,
    ScopedModels,
    Export,
    Import,
    Share,
    Copy,
    Name,
    Session,
    Changelog,
    Hotkeys,
    Fork,
    Clone,
    Tree,
    Trust,
    Login,
    Logout,
    New,
    Compact,
    Resume,
    Reload,
    Quit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) struct BuiltinInvocation<'a> {
    pub(in crate::app) command: BuiltinSlashCommand,
    pub(in crate::app) name: &'static str,
    pub(in crate::app) arguments: Option<&'a str>,
}

#[derive(Clone, Copy)]
struct BuiltinSpec {
    command: BuiltinSlashCommand,
    name: &'static str,
    description: &'static str,
    accepts_arguments: bool,
}

const BUILTINS: &[BuiltinSpec] = &[
    builtin(
        BuiltinSlashCommand::Settings,
        "settings",
        "Open settings menu",
        false,
    ),
    builtin(BuiltinSlashCommand::Model, "model", "Select model", true),
    builtin(
        BuiltinSlashCommand::ScopedModels,
        "scoped-models",
        "Enable or disable models for model cycling",
        false,
    ),
    builtin(
        BuiltinSlashCommand::Export,
        "export",
        "Export session",
        true,
    ),
    builtin(
        BuiltinSlashCommand::Import,
        "import",
        "Import a JSONL session",
        true,
    ),
    builtin(
        BuiltinSlashCommand::Share,
        "share",
        "Share session as a secret GitHub gist",
        false,
    ),
    builtin(
        BuiltinSlashCommand::Copy,
        "copy",
        "Copy last agent message",
        false,
    ),
    builtin(
        BuiltinSlashCommand::Name,
        "name",
        "Set session display name",
        true,
    ),
    builtin(
        BuiltinSlashCommand::Session,
        "session",
        "Show session information and stats",
        false,
    ),
    builtin(
        BuiltinSlashCommand::Changelog,
        "changelog",
        "Show changelog entries",
        false,
    ),
    builtin(
        BuiltinSlashCommand::Hotkeys,
        "hotkeys",
        "Show keyboard shortcuts",
        false,
    ),
    builtin(
        BuiltinSlashCommand::Fork,
        "fork",
        "Fork from a previous user message",
        false,
    ),
    builtin(
        BuiltinSlashCommand::Clone,
        "clone",
        "Duplicate the current session",
        false,
    ),
    builtin(
        BuiltinSlashCommand::Tree,
        "tree",
        "Navigate the session tree",
        false,
    ),
    builtin(
        BuiltinSlashCommand::Trust,
        "trust",
        "Save the project trust decision",
        false,
    ),
    builtin(
        BuiltinSlashCommand::Login,
        "login",
        "Authenticate through Pi in a terminal",
        true,
    ),
    builtin(
        BuiltinSlashCommand::Logout,
        "logout",
        "Remove provider authentication",
        false,
    ),
    builtin(
        BuiltinSlashCommand::New,
        "new",
        "Start a new session",
        false,
    ),
    builtin(
        BuiltinSlashCommand::Compact,
        "compact",
        "Manually compact session context",
        true,
    ),
    builtin(
        BuiltinSlashCommand::Resume,
        "resume",
        "Resume a different session",
        false,
    ),
    builtin(
        BuiltinSlashCommand::Reload,
        "reload",
        "Reload extensions, skills, prompts, and context",
        false,
    ),
    builtin(BuiltinSlashCommand::Quit, "quit", "Quit Pi", false),
];

const fn builtin(
    command: BuiltinSlashCommand,
    name: &'static str,
    description: &'static str,
    accepts_arguments: bool,
) -> BuiltinSpec {
    BuiltinSpec {
        command,
        name,
        description,
        accepts_arguments,
    }
}

pub(in crate::app) fn builtin_invocation(input: &str) -> Option<BuiltinInvocation<'_>> {
    let body = input.strip_prefix('/')?;
    let (name, arguments) = body
        .split_once(' ')
        .map_or((body, None), |(name, arguments)| (name, Some(arguments)));
    let spec = BUILTINS.iter().find(|spec| spec.name == name)?;
    if arguments.is_some() && !spec.accepts_arguments {
        return None;
    }
    Some(BuiltinInvocation {
        command: spec.command,
        name: spec.name,
        arguments: arguments.map(str::trim).filter(|value| !value.is_empty()),
    })
}

pub(in crate::app) fn exact<'a>(
    input: &str,
    commands: &'a [SlashCommand],
) -> Option<&'a SlashCommand> {
    let command_name = command_name(input)?;
    commands.iter().find(|command| command.name == command_name)
}

pub(in crate::app) fn is_exact_for_harness(
    input: &str,
    commands: &[SlashCommand],
    harness: &str,
) -> bool {
    builtin_invocation(input)
        .is_some_and(|invocation| available_for_harness(invocation.command, harness))
        || exact(input, commands).is_some()
}

pub(in crate::app) fn suggestions_for_harness(
    input: &str,
    commands: &[SlashCommand],
    harness: &str,
) -> Vec<ComposerSuggestion> {
    suggestions_matching(input, commands, |command| {
        available_for_harness(command, harness)
    })
}

#[cfg(test)]
pub(in crate::app) fn suggestions(
    input: &str,
    commands: &[SlashCommand],
) -> Vec<ComposerSuggestion> {
    suggestions_matching(input, commands, |_| true)
}

fn suggestions_matching(
    input: &str,
    commands: &[SlashCommand],
    available: impl Fn(BuiltinSlashCommand) -> bool,
) -> Vec<ComposerSuggestion> {
    let Some(query) = input.strip_prefix('/') else {
        return Vec::new();
    };
    if query.chars().any(char::is_whitespace) {
        return Vec::new();
    }
    let mut matches = Vec::new();
    for command in BUILTINS
        .iter()
        .filter(|command| available(command.command))
        .filter(|command| command.name.starts_with(query))
    {
        matches.push(ComposerSuggestion {
            name: command.name.into(),
            description: Some(command.description.into()),
            sigil: '/',
        });
    }
    for command in commands {
        if command.source == SlashCommandSource::Extension
            && command.name.starts_with(query)
            && !matches.iter().any(|existing| existing.name == command.name)
        {
            matches.push(ComposerSuggestion {
                name: command.name.clone(),
                description: command.description.clone(),
                sigil: '/',
            });
        }
    }
    matches
}

pub(in crate::app) fn available_for_harness(command: BuiltinSlashCommand, harness: &str) -> bool {
    harness == "pi"
        || !matches!(
            command,
            BuiltinSlashCommand::Export
                | BuiltinSlashCommand::Import
                | BuiltinSlashCommand::Share
                | BuiltinSlashCommand::Login
                | BuiltinSlashCommand::Logout
                | BuiltinSlashCommand::Reload
        )
}

pub(in crate::app) fn submits_after_completion(name: &str) -> bool {
    BUILTINS
        .iter()
        .find(|command| command.name == name)
        .is_some_and(|command| !command.accepts_arguments)
}

pub(in crate::app) fn is_immediate_extension(input: &str, commands: &[SlashCommand]) -> bool {
    builtin_invocation(input).is_none()
        && exact(input, commands)
            .is_some_and(|command| command.source == SlashCommandSource::Extension)
}

fn command_name(input: &str) -> Option<&str> {
    let command = input.strip_prefix('/')?;
    let name = command.split_once(' ').map_or(command, |(name, _)| name);
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(name: &str, source: SlashCommandSource) -> SlashCommand {
        SlashCommand {
            name: name.into(),
            description: None,
            source,
        }
    }

    #[test]
    fn execution_requires_an_exact_catalog_name() {
        let commands = vec![
            command("extension-reload", SlashCommandSource::Extension),
            command("review-loop", SlashCommandSource::Prompt),
        ];

        assert_eq!(exact("/extension-reload", &commands), Some(&commands[0]));
        assert_eq!(
            exact("/extension-reload now", &commands),
            Some(&commands[0])
        );
        assert_eq!(exact("/review-loop", &commands), Some(&commands[1]));
        assert_eq!(exact("/re", &commands), None);
        assert_eq!(exact("/reload-more", &commands), None);
        assert_eq!(exact("/missing", &commands), None);
        assert_eq!(exact("explain /reload", &commands), None);
        assert_eq!(exact("/extension-reload\nnow", &commands), None);
        assert!(is_immediate_extension("/extension-reload", &commands));
        assert!(!is_immediate_extension("/review-loop", &commands));
    }

    #[test]
    fn external_harnesses_hide_pi_only_commands() {
        let suggestions = suggestions_for_harness("/", &[], "codex-cli");
        let names = suggestions
            .iter()
            .map(|suggestion| suggestion.name.as_str())
            .collect::<Vec<_>>();
        assert!(names.contains(&"model"));
        assert!(names.contains(&"compact"));
        assert!(!names.contains(&"export"));
        assert!(!names.contains(&"reload"));
    }

    #[test]
    fn every_public_pi_builtin_is_in_the_local_catalog() {
        let names = BUILTINS
            .iter()
            .map(|command| command.name)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "settings",
                "model",
                "scoped-models",
                "export",
                "import",
                "share",
                "copy",
                "name",
                "session",
                "changelog",
                "hotkeys",
                "fork",
                "clone",
                "tree",
                "trust",
                "login",
                "logout",
                "new",
                "compact",
                "resume",
                "reload",
                "quit",
            ]
        );
        for name in names {
            assert!(builtin_invocation(&format!("/{name}")).is_some());
        }
        assert!(builtin_invocation("/debug").is_none());
        assert!(builtin_invocation("/arminsayshi").is_none());
        assert!(builtin_invocation("/dementedelves").is_none());
    }

    #[test]
    fn builtin_argument_rules_match_pi() {
        assert_eq!(
            builtin_invocation("/model anthropic/claude").map(|invocation| invocation.command),
            Some(BuiltinSlashCommand::Model)
        );
        assert_eq!(
            builtin_invocation("/compact focus on code")
                .and_then(|invocation| invocation.arguments),
            Some("focus on code")
        );
        assert!(builtin_invocation("/reload now").is_none());
        assert!(builtin_invocation("/quit now").is_none());
        assert!(builtin_invocation("/re").is_none());
    }

    #[test]
    fn suggestions_merge_builtins_and_rpc_commands_without_duplicates() {
        let commands = vec![
            command("reload", SlashCommandSource::Extension),
            command("review", SlashCommandSource::Prompt),
        ];

        assert_eq!(
            suggestions("/re", &commands)
                .into_iter()
                .map(|command| command.name)
                .collect::<Vec<_>>(),
            ["resume", "reload"]
        );
        assert_eq!(suggestions("/", &[]).len(), 22);
        assert!(suggestions("/re now", &commands).is_empty());
        assert!(suggestions("not a command", &commands).is_empty());
    }
}

use super::*;
use crate::protocol::SlashCommandSource;

fn command(name: &str, source: SlashCommandSource) -> SlashCommand {
    SlashCommand {
        name: name.into(),
        description: None,
        source,
    }
}

#[test]
fn execution_requires_an_exact_backend_catalog_name() {
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
}

#[test]
fn suggestions_only_contain_backend_advertised_commands() {
    let commands = vec![
        command("reload", SlashCommandSource::Extension),
        command("review", SlashCommandSource::Prompt),
        command("skill:search", SlashCommandSource::Skill),
    ];

    assert_eq!(
        suggestions("/re", &commands)
            .into_iter()
            .map(|command| command.name)
            .collect::<Vec<_>>(),
        ["reload", "review"]
    );
    assert!(suggestions("/", &[]).is_empty());
    assert!(suggestions("/settings", &[]).is_empty());
    assert!(suggestions("/re now", &commands).is_empty());
    assert!(suggestions("not a command", &commands).is_empty());
}

use crate::protocol::{SlashCommand, SlashCommandSource};

const SOURCES: &[(&str, &str)] = &[
    ("commit", include_str!("../../../prompts/commit.md")),
    ("simplify", include_str!("../../../prompts/simplify.md")),
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Expansion {
    pub(crate) display: String,
    pub(crate) message: String,
    pub(crate) resolution: String,
}

pub(crate) fn commands() -> Vec<SlashCommand> {
    SOURCES
        .iter()
        .map(|(name, source)| {
            let (description, _) = parse(source);
            SlashCommand {
                name: (*name).into(),
                description,
                source: SlashCommandSource::Prompt,
            }
        })
        .collect()
}

pub(crate) fn expand(input: &str) -> Option<Expansion> {
    let mut selected = Vec::new();
    let mut arguments = Vec::new();
    let mut cursor = 0;
    for (start, end, token) in tokens(input) {
        let Some(name) = token.strip_prefix('$') else {
            continue;
        };
        let name = name.strip_prefix("prompt:").unwrap_or(name);
        let Some((_, source)) = SOURCES.iter().find(|(candidate, _)| *candidate == name) else {
            continue;
        };
        let argument = input[cursor..start].trim();
        if !argument.is_empty() {
            arguments.push(argument);
        }
        selected.push(parse(source).1);
        cursor = end;
    }
    if selected.is_empty() {
        return None;
    }
    let trailing = input[cursor..].trim();
    if !trailing.is_empty() {
        arguments.push(trailing);
    }
    let arguments = arguments.join(" ");
    let mut parts = selected;
    if !arguments.is_empty() {
        parts.push(arguments);
    }
    let resolution = parts.join("\n\n");
    Some(Expansion {
        display: input.into(),
        message: resolution.clone(),
        resolution,
    })
}

fn parse(source: &'static str) -> (Option<String>, String) {
    let Some(rest) = source.strip_prefix("---\n") else {
        return (None, source.trim().into());
    };
    let Some((frontmatter, body)) = rest.split_once("\n---\n") else {
        return (None, source.trim().into());
    };
    let description = frontmatter.lines().find_map(|line| {
        line.strip_prefix("description:")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    });
    (description, body.trim().into())
}

fn tokens(input: &str) -> impl Iterator<Item = (usize, usize, &str)> {
    let mut start = None;
    input
        .char_indices()
        .chain(std::iter::once((input.len(), ' ')))
        .filter_map(move |(index, character)| {
            if character.is_whitespace() {
                let token_start = start.take()?;
                Some((token_start, index, &input[token_start..index]))
            } else {
                start.get_or_insert(index);
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_comes_from_the_checked_in_fragments() {
        assert_eq!(
            commands()
                .into_iter()
                .map(|command| (command.name, command.description))
                .collect::<Vec<_>>(),
            [
                (
                    "commit".into(),
                    Some("Commit your changes as whole files".into())
                ),
                (
                    "simplify".into(),
                    Some("Refine your implementation without changing behavior".into())
                )
            ]
        );
    }

    #[test]
    fn fragments_compose_in_user_order_and_keep_other_text() {
        let expansion = expand("please $simplify this $commit with focused tests")
            .expect("owned fragments should expand");
        assert_eq!(
            expansion.display,
            "please $simplify this $commit with focused tests"
        );
        assert!(expansion.message.starts_with("Refine your implementation"));
        assert!(expansion.message.contains("Commit your changes."));
        assert!(
            expansion
                .message
                .ends_with("please this with focused tests")
        );
    }

    #[test]
    fn unknown_and_escaped_invocations_stay_plain() {
        assert!(expand("$missing").is_none());
        assert!(expand(r"\$commit").is_none());
        assert!(expand("cost $100").is_none());
    }

    #[test]
    fn source_qualified_prompt_invocations_expand() {
        assert_eq!(
            expand("$prompt:commit")
                .expect("qualified prompt should expand")
                .display,
            "$prompt:commit"
        );
    }
}

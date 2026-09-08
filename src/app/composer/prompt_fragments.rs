use crate::protocol::{SlashCommand, SlashCommandSource};

const SOURCES: &[(&str, &str)] = &[
    ("commit", include_str!("../../../prompts/commit.md")),
    ("simplify", include_str!("../../../prompts/simplify.md")),
    (
        "simplify-commit",
        include_str!("../../../prompts/simplify-commit.md"),
    ),
    ("show-me", include_str!("../../../prompts/show-me.md")),
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
    let resolution = expand_body(input, SOURCES, &mut Vec::new())?;
    Some(Expansion {
        display: input.into(),
        message: resolution.clone(),
        resolution,
    })
}

fn expand_body(
    input: &str,
    sources: &[(&str, &'static str)],
    active: &mut Vec<usize>,
) -> Option<String> {
    let mut resolution = String::new();
    let mut cursor = 0;
    for (start, _, token) in tokens(input) {
        let token = invocation_token(token);
        let Some(name) = token.strip_prefix('$') else {
            continue;
        };
        let name = name.strip_prefix("prompt:").unwrap_or(name);
        let Some(index) = sources.iter().position(|(candidate, _)| *candidate == name) else {
            continue;
        };
        // Leave cyclic references literal, as with unknown or escaped invocations.
        if active.contains(&index) {
            continue;
        }
        active.push(index);
        let body = parse(sources[index].1).1;
        let expanded = expand_body(&body, sources, active).unwrap_or(body);
        active.pop();
        resolution.push_str(&input[cursor..start]);
        resolution.push_str(&expanded);
        cursor = start + token.len();
    }
    if cursor == 0 {
        return None;
    }
    resolution.push_str(&input[cursor..]);
    Some(resolution)
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

pub(crate) fn invocation_token(token: &str) -> &str {
    token.trim_end_matches(['.', ',', ';', ':', '!', '?'])
}

pub(super) fn tokens(input: &str) -> impl Iterator<Item = (usize, usize, &str)> {
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
#[path = "prompt_fragments_tests.rs"]
mod tests;

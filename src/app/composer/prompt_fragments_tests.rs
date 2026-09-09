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
                ),
                (
                    "simplify-commit".into(),
                    Some("Simplify your changes, then commit them".into())
                ),
                (
                    "show-me".into(),
                    Some("Help the user understand the current topic visually with concise diagrams, code-shape sketches, and focused HTML artifacts.".into())
                )
            ]
        );
}

#[test]
fn fragments_compose_in_user_order_and_keep_other_text() {
    let simplify = parse(include_str!("../../../prompts/simplify.md")).1;
    let commit = parse(include_str!("../../../prompts/commit.md")).1;
    for (input, expected) in [
        ("$simplify and $commit", format!("{simplify} and {commit}")),
        (
            "$simplify-commit",
            format!("{simplify}\n\nThen:\n\n{commit}"),
        ),
        (
            "Please $prompt:simplify-commit! Then $commit",
            format!("Please {simplify}\n\nThen:\n\n{commit}! Then {commit}"),
        ),
        (
            "please $simplify this $commit with focused tests",
            format!("please {simplify} this {commit} with focused tests"),
        ),
        (
            "  $prompt:commit\n\tthen $simplify!\n",
            format!("  {commit}\n\tthen {simplify}!\n"),
        ),
        (
            r"$missing $commit and \$simplify $100",
            format!(r"$missing {commit} and \$simplify $100"),
        ),
    ] {
        let expansion = expand(input).expect("owned fragments should expand");
        assert_eq!(expansion.display, input);
        assert_eq!(expansion.message, expected);
        assert_eq!(expansion.resolution, expected);
    }
}

#[test]
fn nested_expansion_preserves_literals_and_allows_repeated_references() {
    let sources = [
        ("outer", r"$prompt:inner! $inner $missing \$inner"),
        ("inner", "$leaf"),
        ("leaf", "done"),
    ];
    assert_eq!(
        expand_body("$outer", &sources, &mut Vec::new()).expect("test operation should succeed"),
        r"done! done $missing \$inner"
    );
}

#[test]
fn cycles_stop_at_the_repeated_reference() {
    for sources in [vec![("a", "$a")], vec![("a", "$b"), ("b", "$a")]] {
        assert_eq!(
            expand_body("$a! $a", &sources, &mut Vec::new())
                .expect("test operation should succeed"),
            "$a! $a"
        );
    }
}

#[test]
fn show_me_expands_without_attribution_metadata() {
    for input in ["$show-me", "$prompt:show-me"] {
        let expansion = expand(input).expect("show-me prompt should expand");
        let (_, body) = include_str!("../../../prompts/show-me.md")
            .strip_prefix("---\n")
            .expect("test operation should succeed")
            .split_once("\n---\n")
            .expect("test operation should succeed");
        assert_eq!(expansion.message, body.trim());
    }
}

#[test]
fn unknown_and_escaped_invocations_stay_plain() {
    assert!(expand("$missing").is_none());
    assert!(expand(r"\$commit").is_none());
    assert!(expand("cost $100").is_none());
}

#[test]
fn trailing_punctuation_expands_and_is_preserved() {
    for suffix in [".", ",", ";", ":", "!", "?", "..."] {
        for name in ["commit", "prompt:commit"] {
            let input = format!("${name}{suffix}");
            let expansion = expand(&input).expect("punctuated prompt should expand");
            assert_eq!(expansion.display, input);
            let body = parse(include_str!("../../../prompts/commit.md")).1;
            assert_eq!(expansion.message, format!("{body}{suffix}"));
        }
    }
    assert!(expand("$commit.md").is_none());
    assert!(expand(r"\$commit.").is_none());
    assert!(expand("$commit-extra.").is_none());
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

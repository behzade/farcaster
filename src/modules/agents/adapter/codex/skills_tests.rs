use super::*;

fn skill(name: &str, path: &str, enabled: bool) -> Value {
    json!({"name": name, "path": path, "enabled": enabled, "description": "Review code"})
}

#[test]
fn catalog_excludes_disabled_foreign_and_ambiguous_skills() {
    let skills = Skills::parse(
        json!({"data": [
            {"cwd":"/elsewhere", "skills":[skill("foreign", "/foreign/SKILL.md", true)]},
            {"cwd":"/project", "skills":[
                skill("review", "/skills/review/SKILL.md", true),
                skill("review", "/skills/review/SKILL.md", true),
                skill("disabled", "/skills/disabled/SKILL.md", false),
                skill("relative", "relative/SKILL.md", true),
                skill("ambiguous", "/one/SKILL.md", true),
                skill("ambiguous", "/two/SKILL.md", true)
            ]}
        ]}),
        Path::new("/project"),
    )
    .unwrap();
    assert_eq!(
        skills.commands(),
        [json!({"name":"skill:review", "description":"Review code", "source":"skill"})]
    );
}

#[test]
fn invocations_attach_exact_paths_and_preserve_other_text() {
    let skills = Skills::parse(
        json!({"data":[{"cwd":"/project", "skills":[
            skill("review", "/skills/review/SKILL.md", true),
            skill("check", "/skills/check/SKILL.md", true)
        ]}]}),
        Path::new("/project"),
    )
    .unwrap();
    let message = "/skill:review Fix this\n$skill:check, $review! $unknown \\$review word$review $review.md  ";
    assert_eq!(
        serde_json::to_value(skills.input(message.into())).unwrap(),
        json!([
            {"type":"text", "text":"$review Fix this\n$check, $review! $unknown \\$review word$review $review.md  ", "text_elements":[]},
            {"type":"skill", "name":"review", "path":"/skills/review/SKILL.md"},
            {"type":"skill", "name":"check", "path":"/skills/check/SKILL.md"}
        ])
    );
}

#[test]
fn unavailable_or_malformed_catalog_does_not_break_startup() {
    use std::io::Cursor;
    for response in [
        json!({"id":1,"error":{"code":-32601,"message":"unknown method"}}),
        json!({"id":1,"result":{}}),
    ] {
        let mut connection = CodexConnection::new(Cursor::new(format!("{response}\n")), Vec::new());
        assert!(
            Skills::load(&mut connection, Path::new("/project"))
                .commands()
                .is_empty()
        );
    }
}

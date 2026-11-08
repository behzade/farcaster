use super::*;

#[test]
fn unwraps_one_literal_shell_layer_without_changing_script_text() {
    let scripts = [
        "cargo test --offline",
        "printf '%s\\n' \"$HOME\"; git diff --check",
        "echo first\nprintf 'متن'\n",
        "sh -c 'echo nested'",
    ];
    for shell in [
        "/run/current-system/sw/bin/sh",
        "/bin/bash",
        "zsh",
        "dash",
        "ksh",
    ] {
        for flags in ["-c", "-lc", "-cl"] {
            for script in scripts {
                let invocation = shell_words::join([shell, flags, script]);
                assert_eq!(display(&invocation), script, "{invocation}");
            }
        }
    }
    assert_eq!(display("/bin/sh -lc \"echo hello\""), "echo hello");
    assert_eq!(display("sh -c pwd"), "pwd");
}

#[test]
fn keeps_ambiguous_or_semantically_significant_wrappers_intact() {
    for command in [
        "cargo test",
        "sudo sh -c 'pwd'",
        "env HOME=/tmp sh -c 'pwd'",
        "sh -ec 'one; two'",
        "sh -c 'echo $1' arg0 value",
        "sh -c 'echo hi' > output.txt",
        "sh -c 'echo hi'; echo next",
        "sh -c 'echo hi'\necho next",
        "sh -c 'echo hi' # comment",
        "sh -c \"echo $HOME\"",
        "sh -c \"echo `pwd`\"",
        "sh -c $(cat script)",
        "sh -c ~/script",
        "sh -c *",
        "sh -c ''",
        "sh -c 'unterminated",
        "fish -c 'echo hi'",
        "python -c 'print(1)'",
    ] {
        assert_eq!(display(command), command);
    }
    let oversized = format!("sh -c '{}'", "x".repeat(64 * 1024));
    assert!(matches!(display(&oversized), Cow::Borrowed(_)));
}

# Shell environment capture contract

Farcaster captures the account's interactive login-shell environment before
opening its window, then relaunches with that environment. App capture runs in
HOME; project capture runs in the requested project directory.

## Requirements

Bash, Zsh, and fish must retain:

- Login and interactive initialization, with terminal stdin/stdout.
- First-prompt hooks and their exports, including project-local PATH entries.
  Using `-c` is not acceptable if it skips those hooks.
- The requested working directory, including paths containing spaces.
- Prompt exports overriding inherited/config-time values.
- Exported empty strings, newlines, Unicode, and embedded `=`.
- TERM and shell feature settings, without capture-specific overrides.
- Environment records unpolluted by configuration/prompt chatter.
- Exact captured PATH in the project PATH handoff, usable by child processes
  to resolve project-local executables.

Explicit Farcaster launch overrides (such as FARCASTER_PI_PATH) must survive app
relaunch while retaining the shell-derived PATH.

## Terminal-query regression

With fish 4.7.1, the unanswered Primary Device Attribute query delayed capture by
about ten seconds, even with `--no-config`. See fish's
[terminal compatibility contract](https://fishshell.com/docs/4.7/terminal-compatibility.html).

Capture drains PTY output and replies to `ESC [ c` and `ESC [ 0 c` with
`ESC [ ? 0 c`, completing fish's query barrier without advertising optional
capabilities. Other queries are ignored. The responder handles split reads and
stops at the environment start marker: exported escape sequences remain data.
Shell invocation, prompt hooks, capture command, parsing, and PATH handoff are
unchanged. This is not a full terminal emulator.

## Tests

Normal tests include a real-PTY handshake fixture without requiring fish:

```sh
cargo test --bin farcaster shell_environment -- --nocapture
```

Real-shell fixtures in
`src/modules/agents/adapter/shell_environment_integration_tests.rs` use isolated
HOME/configuration, Bash PROMPT_COMMAND, Zsh precmd, and fish's fish_prompt event.
They verify exports, executable discovery by a child, and capture under five
seconds—a timeout-regression ceiling, not a benchmark.

These three tests require the named shells and `script` on PATH and are ignored
by default. Missing prerequisites fail rather than silently skipping:

```sh
cargo test --bin farcaster shell_environment::tests::integration -- --ignored --nocapture --test-threads=1
```

Filter on `real_bash_capture_`, `real_zsh_capture_`, or `real_fish_capture_` to test
one shell. Use an outer runner timeout: capture has no overall timeout yet.
The fixtures do not exercise real direnv/Nix integration, user configuration,
agent backends, or the full GUI relaunch.

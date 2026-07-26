# Changelog

## [Unreleased]

## [0.0.10] - 2026-06-18

### Added
- Add the standalone Pi Assistant extension package and bundle it into the umbrella `@yevhen.b/bo-pi` package.
- Split docs so each extension now has its own README.
- Add dedicated macOS Theme Sync guide and link both extensions from the root README.
- Add extension-local `package.json`, `CHANGELOG.md`, and `LICENSE` files for the top-level `preflight/` and `macos-theme-sync/` directories.
- Add the `/assistant` command for copying the last completed assistant response to the clipboard as Markdown, plain text, or macOS rich text, and for saving/appending it to files.

### Changed
- Reframe the repo around standalone extension packages plus the existing umbrella `@yevhen.b/bo-pi` package.
- Move shipped extensions to top-level directories (`preflight/`, `macos-theme-sync/`) to match the multi-extension repo structure.
- Publish packages by tag (`bo-pi/v*`, `preflight/v*`, `pi-assistant/v*`, `macos-theme-sync/v*`) from the matching directory.
- Keep the root package as a publishable umbrella bundle while standalone packages remain independently installable.
- Validate published package contents with `npm pack --dry-run` in the release workflow.
- Switch Pi Assistant Markdown conversion to the `remark` / `unified` stack for HTML and plain-text rendering, including GFM support.
- Migrate every extension package to the latest `@earendil-works/pi-*` SDK package names.
- Update preflight model auth resolution to use `getApiKeyAndHeaders()` so latest Pi provider headers and provider-scoped env continue to work.
- Declare `@earendil-works/pi-tui` peer dependencies for packages that use Pi UI types or components.

### Fixed
- Clear legacy npm auth tokens in the publish workflow so npm trusted publishing (OIDC) is used consistently.
- Use Node 22 in the publish workflow so npm trusted publishing uses a compatible npm CLI.
- Add explicit OIDC environment checks in the publish workflow to fail fast when runner OIDC context is unavailable.
- Upgrade npm to v11 in CI before publish so trusted publishing is available on GitHub runners.

## [0.0.9] - 2026-03-01
### Added
- GitHub Actions publish workflow that releases to npm on `bo-pi/v*` tag pushes using npm trusted publishing (OIDC).

### Changed
- Releasing guide now documents tag-driven automated publish and one-time trusted publisher setup.

## [0.0.8] - 2026-03-01
### Changed
- Skip preflight model calls when deterministic `allow` or `deny` rules already decide the tool call.
- Strengthen the preflight prompt with required tool-call IDs, stricter field requirements, and a response skeleton for JSON output.

### Fixed
- Retry failed preflight parsing/normalization up to two additional silent attempts before showing failure handling.

## [0.0.7] - 2026-02-21
### Added
- Warn on conflicting custom rules before save, with options to edit (default), save anyway, or cancel.
- Rule consistency evaluator for candidate custom rules with non-blocking fallback when checks fail.

### Changed
- Rule suggestions now include existing policy + deterministic permission context (including wildcard/global rules).
- Rule suggestion filtering now removes near-duplicates against existing policy rules.

## [0.0.6] - 2026-02-17
### Fixed
- Ask for confirmation in the approval UI when policy rules block a command (for example `git clone`) instead of auto-rejecting.

## [0.0.5] - 2026-02-17
### Fixed
- Re-enable `Tab` suggestion accept/next after empty custom-rule submission when suggestions are available.

## [0.0.4] - 2026-02-17
### Fixed
- Drop invalid LLM suggestion artifacts (for example `<Function_calls>`) from the custom-rule row.
- Normalize additional policy verbs (`block`, `forbid`, `require`, `prompt`, `permit`) to canonical `Allow`/`Ask`/`Deny` suggestions.
- Show a muted `Type custom rule` fallback hint when no suggestion is available.

## [0.0.3] - 2026-02-14
### Added
- Custom rule row in approval UI: bo-pi suggests rules, accept with Tab, cycle alternatives, or type your own.
- Custom rules are applied immediately to the current tool call (allow/deny/re-ask).
- Debug file logging to `.pi/preflight/logs/preflight-debug.log`.
- Tests for rule suggestion normalization, approval flow, and persistence.

### Changed
- Tab is now the only shortcut for accepting and cycling rule suggestions (removed Ctrl+N).
- Rule suggestions are hardened: headings, trailing punctuation, and duplicate lines are filtered.
- Suggestion text requires explicit Tab acceptance; Enter does not implicitly accept.
- Updated README and preflight docs for current behavior.

## [0.0.2] - 2026-02-13
### Added
- Document release process, npm publishing auth, and preflight usage.

### Changed
- Publish package under the @yevhen.b scope.
- Fix package scope in README install commands.

## [0.0.1] - 2026-02-11
### Added
- Initial bo-pi preflight extension with approvals, policies, and rules.

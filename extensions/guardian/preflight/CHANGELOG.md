# Changelog

## [Unreleased]

### Added
- Review explicit mixed sibling verdicts in one request and cache each result by tool-call ID.
- Hand exact one-shot outside-write grants to the sandbox after review.

### Changed
- Ignore workspace permission files until Pi trusts the project.
- Pass Pi cancellation to reviewer calls and retry waits.

### Fixed
- Expand home-relative paths before guarding built-in file tools.
- Check both lexical and resolved paths so symlinks cannot bypass protected-path rules.

## [0.0.10] - 2026-06-18

### Changed
- Move Preflight into a top-level extension directory in the shared bo-pi repo.
- Add extension-local README and package metadata.
- Migrate Preflight to the latest `@earendil-works/pi-*` SDK packages.
- Resolve model auth via `getApiKeyAndHeaders()` so latest Pi custom provider headers and env are forwarded during preflight calls.

## [0.0.9] - 2026-03-11
### Added
- Preflight is now documented as a standalone extension inside the multi-extension bo-pi package.

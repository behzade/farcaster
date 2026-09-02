PROJECT ?= $(CURDIR)
CARGO_TARGET_DIR ?= $(CURDIR)/target
LOG_LINES ?= 50
DEFAULT_FARCASTER_DATA_DIR := $(if $(XDG_DATA_HOME),$(XDG_DATA_HOME),$(HOME)/.local/share)/farcaster
LOG_FILE ?= $(if $(FARCASTER_DATA_DIR),$(FARCASTER_DATA_DIR),$(DEFAULT_FARCASTER_DATA_DIR))/logs/farcaster.log
TAIL_ARGS ?= -n $(LOG_LINES)

.PHONY: run test e2e debug release release-debug bundle bundle-relaunch package logs check check-flake

run:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo run -- "$(PROJECT)"

test:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo test

# Runs live agent conformance. Set HARNESS to pi, codex-cli, cursor-cli, or opencode2.
e2e:
	$(if $(HARNESS),FARCASTER_E2E_HARNESS="$(HARNESS)" )CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" \
		cargo test live_harnesses_conform_to_session_outcomes -- --ignored --nocapture

debug:
	DEBUG=true CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo run -- "$(PROJECT)"

release:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo run --release -- "$(PROJECT)"

release-debug:
	DEBUG=true CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo run --release -- "$(PROJECT)"

bundle:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" BUNDLE_FORMATS="$(BUNDLE_FORMATS)" PROJECT="$(PROJECT)" ./scripts/bundle.sh

bundle-relaunch:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" BUNDLE_FORMATS="$(BUNDLE_FORMATS)" PROJECT="$(PROJECT)" ./scripts/bundle.sh --relaunch

package:
	@test -n "$(FORMAT)" || (echo "usage: make package FORMAT=app|dmg|appimage|deb|pacman" >&2; exit 1)
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo packager --release --formats "$(FORMAT)"

# Override LOG_LINES, TAIL_ARGS (for example, "-n 100 -f"), or LOG_FILE.
logs:
	@tail $(TAIL_ARGS) "$(LOG_FILE)"

check:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo fmt --check
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo test
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo check
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo clippy --all-targets -- -D warnings

check-flake:
	nix flake check

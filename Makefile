PROJECT ?= $(CURDIR)
CARGO_TARGET_DIR ?= $(CURDIR)/target
LOG_LINES ?= 50
DEFAULT_FARCASTER_DATA_DIR := $(if $(XDG_DATA_HOME),$(XDG_DATA_HOME),$(HOME)/.local/share)/farcaster
LOG_FILE ?= $(if $(FARCASTER_DATA_DIR),$(FARCASTER_DATA_DIR),$(DEFAULT_FARCASTER_DATA_DIR))/logs/farcaster.log
TAIL_ARGS ?= -n $(LOG_LINES)

.PHONY: run test debug release release-debug bundle-macos package package-linux logs check check-flake

run:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo run -- "$(PROJECT)"

test:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo test

debug:
	DEBUG=true CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo run -- "$(PROJECT)"

release:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo run --release -- "$(PROJECT)"

release-debug:
	DEBUG=true CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo run --release -- "$(PROJECT)"

bundle-macos:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" ./scripts/bundle-macos.sh

package:
	@test -n "$(FORMAT)" || (echo "usage: make package FORMAT=appimage|deb|pacman" >&2; exit 1)
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo packager --release --formats "$(FORMAT)"

package-linux:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo packager --release --formats appimage,deb,pacman

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

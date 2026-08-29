PROJECT ?= $(CURDIR)
CARGO_TARGET_DIR ?= $(CURDIR)/target

.PHONY: run test debug release release-debug check check-flake

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

check:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo fmt --check
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo test
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo check
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo clippy --all-targets -- -D warnings

check-flake:
	nix flake check

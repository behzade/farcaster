PROJECT ?= $(CURDIR)
CARGO_TARGET_DIR ?= $(CURDIR)/target

.PHONY: run debug check-gpui
run:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo run --manifest-path apps/pi-gpui/Cargo.toml -- "$(PROJECT)"

debug:
	DEBUG=true CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo run --manifest-path apps/pi-gpui/Cargo.toml -- "$(PROJECT)"

check-gpui:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo fmt --manifest-path apps/pi-gpui/Cargo.toml --check
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo test --manifest-path apps/pi-gpui/Cargo.toml
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo check --manifest-path apps/pi-gpui/Cargo.toml
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo clippy --manifest-path apps/pi-gpui/Cargo.toml --all-targets -- -D warnings

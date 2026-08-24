PROJECT ?= $(CURDIR)
CARGO_TARGET_DIR ?= $(CURDIR)/target
PI_GPUI_DEPS_ROOT ?= $(CURDIR)/result-pi-gpui-deps

.PHONY: run debug release release-debug root-gpui-deps check-gpui check-flake update-pi-nono
run:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo run --manifest-path apps/pi-gpui/Cargo.toml -- "$(PROJECT)"
test:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo test --manifest-path apps/pi-gpui/Cargo.toml -- "$(PROJECT)"

debug:
	DEBUG=true CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo run --manifest-path apps/pi-gpui/Cargo.toml -- "$(PROJECT)"

release:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo run --release --manifest-path apps/pi-gpui/Cargo.toml -- "$(PROJECT)"

release-debug:
	DEBUG=true CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo run --release --manifest-path apps/pi-gpui/Cargo.toml -- "$(PROJECT)"

root-gpui-deps:
	nix build .#pi-gpui-deps --out-link "$(PI_GPUI_DEPS_ROOT)"

check-gpui:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo fmt --manifest-path apps/pi-gpui/Cargo.toml --check
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo test --manifest-path apps/pi-gpui/Cargo.toml
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo check --manifest-path apps/pi-gpui/Cargo.toml
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" cargo clippy --manifest-path apps/pi-gpui/Cargo.toml --all-targets -- -D warnings

check-flake:
	nix flake check

update-pi-nono:
	nix flake update piNono
	$(MAKE) check-flake

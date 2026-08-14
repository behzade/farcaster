PROJECT ?= $(CURDIR)
CARGO_TARGET_DIR ?= $(CURDIR)/target

.PHONY: run
run:
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" nix develop .#pi-gpui -c cargo run --manifest-path apps/pi-gpui/Cargo.toml -- "$(PROJECT)"

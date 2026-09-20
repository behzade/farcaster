PROJECT ?= $(CURDIR)
export CARGO_TARGET_DIR ?= $(CURDIR)/target
DATA_DIR := $(if $(FARCASTER_DATA_DIR),$(FARCASTER_DATA_DIR),$(if $(XDG_DATA_HOME),$(XDG_DATA_HOME),$(HOME)/.local/share)/farcaster)
LOG_FILE ?= $(DATA_DIR)/logs/farcaster.log
TAIL_ARGS ?= -n 50
BUMP ?= patch
DEP_GRAPH := Cargo.toml Cargo.lock
INCREMENTAL_STAMP := $(CARGO_TARGET_DIR)/.incremental-stamp
CARGO_TARGETS := build run test e2e debug release release-debug release-preview release-publish bundle bundle-relaunch package clippy check

.SILENT:
.PHONY: $(CARGO_TARGETS) logs fmt check-flake clean prune-incremental build-nix

$(CARGO_TARGETS): | $(INCREMENTAL_STAMP)

$(INCREMENTAL_STAMP): $(DEP_GRAPH)
	mkdir -p "$(dir $@)"
	if [ -f "$@" ]; then echo "dependency graph changed: pruning the local incremental cache"; cargo clean -p farcaster; fi
	touch "$@"

build:
	cargo build
run debug:
	DEBUG=$(if $(filter debug,$@),true) cargo run -- "$(PROJECT)"
test:
	cargo test
e2e:
	HARNESS="$(HARNESS)" CASE="$(CASE)" sh scripts/e2e.sh
release release-debug:
	DEBUG=$(if $(filter release-debug,$@),true) cargo run --release -- "$(PROJECT)"
release-local:
	cargo --config 'paths = ["$(GPUI_GHOSTTY_DIR)/crates/gpui-ghostty"]' run --release -- "$(PROJECT)"
release-preview release-publish:
	cargo release "$(BUMP)" --package farcaster $(if $(filter release-publish,$@),--execute)
bundle bundle-relaunch:
	BUNDLE_FORMATS="$(BUNDLE_FORMATS)" PROJECT="$(PROJECT)" ./scripts/bundle.sh $(if $(filter bundle-relaunch,$@),--relaunch)
package:
	test -n "$(FORMAT)" || (echo "usage: make package FORMAT=app|dmg|appimage|deb|pacman" >&2; exit 1)
	BUNDLE_FORMATS="$(FORMAT)" ./scripts/bundle.sh
logs:
	tail $(TAIL_ARGS) "$(LOG_FILE)"
fmt:
	cargo fmt
clippy:
	cargo clippy --all-targets -- -D warnings
check:
	cargo fmt --check && cargo test && cargo check && cargo clippy --all-targets -- -D warnings
check-flake:
	nix flake check
build-nix:
	nix build --print-build-logs .#default
clean:
	cargo clean
prune-incremental:
	cargo clean -p farcaster

PROJECT ?= $(CURDIR)
GPUI_GHOSTTY_DIR ?= $(abspath ../gpui-ghostty)
LOG_LINES ?= 50
DEFAULT_FARCASTER_DATA_DIR := $(if $(XDG_DATA_HOME),$(XDG_DATA_HOME),$(HOME)/.local/share)/farcaster
LOG_FILE ?= $(if $(FARCASTER_DATA_DIR),$(FARCASTER_DATA_DIR),$(DEFAULT_FARCASTER_DATA_DIR))/logs/farcaster.log
TAIL_ARGS ?= -n $(LOG_LINES)
BUMP ?= patch

.PHONY: run test measure e2e debug release release-local release-debug release-preview release-publish bundle bundle-relaunch package logs check check-flake build-nix

run:
	cargo run -- "$(PROJECT)"

test:
	sh scripts/test.sh

measure:
	FARCASTER_PERF_TRACE=1 sh scripts/test.sh --bin farcaster -- \
		--nocapture --test-threads=1 switch_perf_tests writer_perf_tests offscreen_perf_tests

e2e:
	HARNESS="$(HARNESS)" CASE="$(CASE)" \
		sh scripts/e2e.sh

debug:
	DEBUG=true cargo run -- "$(PROJECT)"

release:
	cargo run --release -- "$(PROJECT)"

release-local:
	cargo \
		--config 'paths = ["$(GPUI_GHOSTTY_DIR)/crates/gpui-ghostty"]' \
		run --release -- "$(PROJECT)"

release-debug:
	DEBUG=true cargo run --release -- "$(PROJECT)"

release-preview:
	cargo release "$(BUMP)" --package farcaster

release-publish:
	cargo release "$(BUMP)" --package farcaster --execute

bundle:
	BUNDLE_FORMATS="$(BUNDLE_FORMATS)" PROJECT="$(PROJECT)" ./scripts/bundle.sh

bundle-relaunch:
	BUNDLE_FORMATS="$(BUNDLE_FORMATS)" PROJECT="$(PROJECT)" ./scripts/bundle.sh --relaunch

package:
	@test -n "$(FORMAT)" || (echo "usage: make package FORMAT=app|dmg|appimage|deb|pacman" >&2; exit 1)
	BUNDLE_FORMATS="$(FORMAT)" ./scripts/bundle.sh

logs:
	@tail $(TAIL_ARGS) "$(LOG_FILE)"

check:
	cargo fmt --check
	$(MAKE) test
	cargo check
	cargo clippy --all-targets -- -D warnings

check-flake:
	nix flake check

build-nix:
	nix build --print-build-logs .#default

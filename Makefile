PROJECT ?= .

.PHONY: run test check

run:
	cargo run -- "$(PROJECT)"

test:
	cargo test

check:
	cargo fmt --check
	cargo test
	cargo check
	cargo clippy --all-targets -- -D warnings

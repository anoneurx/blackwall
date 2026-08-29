.PHONY: build image run fmt clippy

build:
	cargo build --workspace

image:
	cargo run -p blackwall-runner -- image

run:
	cargo run -p blackwall-runner -- run

fmt:
	cargo fmt --all

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

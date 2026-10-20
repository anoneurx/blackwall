# Run every recipe from the repository root so relative paths (cargo
# manifests, target/) resolve correctly for both `make -f build/Makefile`
# and `make -C build`.
ROOT := $(abspath $(dir $(lastword $(MAKEFILE_LIST)))/..)
RUN = cd $(ROOT) &&

.PHONY: build iso image run fmt clippy

build:
	$(RUN) cargo build --workspace

iso:
	$(RUN) ./build/scripts/build-iso.sh --version v0.1

image:
	$(RUN) cargo run -p blackwall-runner -- image

run:
	$(RUN) cargo run -p blackwall-runner -- run

fmt:
	$(RUN) cargo fmt --all

clippy:
	$(RUN) cargo clippy --workspace --all-targets -- -D warnings

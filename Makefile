.PHONY: build run test fmt clippy clean

build:
	cargo build --workspace
	cd userspace/init && cargo build --target x86_64-unknown-none
	cargo build -p blackwall-kernel --target x86_64-unknown-uefi
	cargo build -p blackwall-bootloader --target x86_64-unknown-uefi
	cargo run -p bw-iso-builder -- --output dist/blackwall-server-v0.1.iso

run:
	@cp /usr/share/OVMF/OVMF_VARS_4M.fd /tmp/blackwall-vars-run.fd 2>/dev/null || true
	@OVMF_CODE=$$(find /usr/share/OVMF -name 'OVMF_CODE*.fd' 2>/dev/null | head -1); \
	qemu-system-x86_64 -enable-kvm -machine q35 -m 256M -nographic \
	  -drive if=pflash,format=raw,unit=0,readonly=on,file=$$OVMF_CODE \
	  -drive if=pflash,format=raw,unit=1,file=/tmp/blackwall-vars-run.fd \
	  -cdrom dist/blackwall-server-v0.1.iso

test:
	cargo test --workspace

fmt:
	cargo fmt --all

clippy:
	cargo clippy --workspace

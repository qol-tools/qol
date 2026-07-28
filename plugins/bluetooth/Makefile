.PHONY: build dev release test check lint lint-fix fmt fmt-check smoke-daemon clean ci-local

BINARY = plugin-bluetooth
OUTPUT = ./$(BINARY)
STAGED = ./$(BINARY).new
TARGET_DIR = ../../target

build:
	cargo build

dev:
	cargo build
	install -m 755 $(TARGET_DIR)/debug/$(BINARY) $(STAGED)
	mv -f $(STAGED) $(OUTPUT)

release:
	cargo build --release
	install -m 755 $(TARGET_DIR)/release/$(BINARY) $(STAGED)
	mv -f $(STAGED) $(OUTPUT)

test:
	cargo test

check:
	cargo check

lint: fmt-check
	cargo clippy --all-targets -- -D warnings

lint-fix: fmt
	cargo clippy --fix --all-targets --allow-dirty --allow-staged -- -D warnings

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all --check

smoke-daemon:
	cargo build -p plugin-bluetooth
	node tools/smoke-daemon.mjs

clean:
	cargo clean

ci-local:
	@echo "==> cargo fmt --check"
	cargo fmt --all -- --check
	@echo "==> RUSTFLAGS=-D warnings cargo clippy --all-targets --all-features --keep-going -- -D warnings"
	RUSTFLAGS="-D warnings" cargo clippy --all-targets --all-features --keep-going -- -D warnings
	@echo "==> RUSTFLAGS=-D warnings cargo test --all-features"
	RUSTFLAGS="-D warnings" cargo test --all-features
	@if rustup target list --installed 2>/dev/null | grep -q '^x86_64-pc-windows-gnu$$'; then \
		echo "==> cargo check --target x86_64-pc-windows-gnu"; \
		RUSTFLAGS="-D warnings" cargo check --target x86_64-pc-windows-gnu --all-features; \
	else \
		echo "==> SKIP cross-check x86_64-pc-windows-gnu (rustup target add x86_64-pc-windows-gnu to enable)"; \
	fi
	@if rustup target list --installed 2>/dev/null | grep -q '^x86_64-apple-darwin$$'; then \
		echo "==> cargo check --target x86_64-apple-darwin"; \
		RUSTFLAGS="-D warnings" cargo check --target x86_64-apple-darwin --all-features; \
	else \
		echo "==> SKIP cross-check x86_64-apple-darwin (rustup target add x86_64-apple-darwin to enable; macOS SDK still needed for full builds)"; \
	fi
	@echo "==> ci-local: ok"

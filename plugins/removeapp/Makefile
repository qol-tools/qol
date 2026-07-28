.PHONY: build dev release test check lint lint-fix fmt fmt-check clean ci-local

BINARY = removeapp
OUTPUT = ./$(BINARY)
STAGED = ./$(BINARY).new

build:
	cargo build

dev:
	cargo build
	install -m 755 target/debug/$(BINARY) $(STAGED)
	mv -f $(STAGED) $(OUTPUT)

release:
	cargo build --release
	install -m 755 target/release/$(BINARY) $(STAGED)
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

clean:
	cargo clean

ci-local:
	@echo "==> cargo fmt --check"
	cargo fmt --all -- --check
	@echo "==> clippy -D warnings"
	RUSTFLAGS="-D warnings" cargo clippy --all-targets --all-features --keep-going -- -D warnings
	@echo "==> test"
	RUSTFLAGS="-D warnings" cargo test --all-features
	@echo "==> ci-local: ok"

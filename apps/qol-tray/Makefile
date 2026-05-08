.PHONY: build release run dev setup test check lint lint-fix fmt fmt-check icons clean install install-dev ci-local sync recompile-linked

DEV_HOOKS := .qol-tray-dev-hooks

setup:
	@bash scripts/dev-setup.sh

$(DEV_HOOKS):
	@bash scripts/dev-setup.sh

build: lint
	cargo build

release:
	@set -e; \
	$(MAKE) lint; \
	cargo test; \
	git fetch --tags --force; \
	OLD=$$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/'); \
	if [ -z "$$OLD" ]; then \
		echo "Failed to parse version from Cargo.toml"; \
		exit 1; \
	fi; \
	MAJOR=$$(echo $$OLD | cut -d. -f1); \
	MINOR=$$(echo $$OLD | cut -d. -f2); \
	PATCH=$$(echo $$OLD | cut -d. -f3); \
	if [ -n "$(RELEASE_VERSION)" ]; then \
		NEW="$(RELEASE_VERSION)"; \
	else \
		LAST_TAG=$$(git describe --tags --abbrev=0 --match 'v[0-9]*' 2>/dev/null || true); \
		RANGE="HEAD"; \
		if [ -n "$$LAST_TAG" ]; then \
			RANGE="$$LAST_TAG..HEAD"; \
		fi; \
		BUMP="patch"; \
		if git log --format='%s' $$RANGE | grep -Eiq '^[a-z0-9_-]+(\([^)]+\))?!:'; then \
			BUMP="major"; \
		elif git log --format='%s' $$RANGE | grep -Eiq '^feat(\([^)]+\))?:'; then \
			BUMP="minor"; \
		fi; \
		if [ "$$BUMP" = "major" ]; then \
			NEW_MAJOR=$$((MAJOR + 1)); \
			NEW_MINOR=0; \
			NEW_PATCH=0; \
		elif [ "$$BUMP" = "minor" ]; then \
			NEW_MAJOR=$$MAJOR; \
			NEW_MINOR=$$((MINOR + 1)); \
			NEW_PATCH=0; \
		else \
			NEW_MAJOR=$$MAJOR; \
			NEW_MINOR=$$MINOR; \
			NEW_PATCH=$$((PATCH + 1)); \
		fi; \
		while git tag -l "v$$NEW_MAJOR.$$NEW_MINOR.$$NEW_PATCH" | grep -q .; do \
			NEW_PATCH=$$((NEW_PATCH + 1)); \
		done; \
		NEW="$$NEW_MAJOR.$$NEW_MINOR.$$NEW_PATCH"; \
	fi; \
	if [ "$$(uname)" = "Darwin" ]; then \
		sed -i '' "s/^version = \"$$OLD\"/version = \"$$NEW\"/" Cargo.toml; \
	else \
		sed -i "s/^version = \"$$OLD\"/version = \"$$NEW\"/" Cargo.toml; \
	fi; \
	git add Cargo.toml; \
	git commit -m "chore(release): v$$NEW"; \
	git tag "v$$NEW"; \
	git push; \
	git push --tags

run: lint
	cargo run --bin qol-tray

sync:
	@if command -v qol-sync >/dev/null 2>&1; then \
		qol-sync; \
	elif [ -x ../qol-cicd/bin/qol-sync ]; then \
		../qol-cicd/bin/qol-sync; \
	else \
		echo "sync: qol-sync not found, skipping"; \
	fi

recompile-linked:
	@bash scripts/recompile-linked-plugins.sh

dev: sync recompile-linked $(DEV_HOOKS) lint
	@if [ -x ./$(DEV_HOOKS) ]; then ./$(DEV_HOOKS); fi
	-pkill -f 'qol-tray'
	cargo run --bin qol-tray --features dev -- --write-mode=dev
	cargo run --bin qol-tray --features dev

test: lint
	cargo test

check: lint
	cargo check

lint: fmt-check
	cargo clippy --all-targets --all-features -- -D warnings

lint-fix: fmt
	cargo clippy --fix --all-targets --all-features --allow-dirty --allow-staged -- -D warnings

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all --check

icons:
	bash scripts/generate-icons.sh

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

install:
	cargo build --release --bins
	target/release/qol-tray-install

install-dev:
	cargo build --release --bins --features dev
	target/release/qol-tray-install --dev


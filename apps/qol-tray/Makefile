.PHONY: build release run dev test check lint lint-fix fmt fmt-check clean contracts install auto-config fx

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

dev: lint
	-pkill -f 'qol-tray'
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

clean:
	cargo clean

contracts:
	python3 scripts/validate_first_party_plugin_contracts.py --plugins-root ../qol-tools

install:
	cargo build --release --bins
	target/release/qol-tray-install

auto-config:
	mkdir -p ui/auto-config
	cp ../qol-config/js/*.js ui/auto-config/

fx:
	cp ../qol-fx/src/*.js ui/lib/

wasm:
	cd ../qol-wasm && wasm-pack build --target web --release
	mkdir -p ui/wasm
	cp ../qol-wasm/pkg/qol_wasm.d.ts ui/wasm/
	cp ../qol-wasm/pkg/qol_wasm_bg.wasm ui/wasm/
	cp ../qol-wasm/pkg/qol_wasm_bg.wasm.d.ts ui/wasm/
	cp ../qol-wasm/pkg/qol_wasm.js ui/wasm/

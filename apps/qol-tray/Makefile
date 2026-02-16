.PHONY: build release run dev test check clean contracts install

build:
	cargo build

release:
	@cargo test && \
	git fetch --tags && \
	OLD=$$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/'); \
	MAJOR=$$(echo $$OLD | cut -d. -f1); \
	MINOR=$$(echo $$OLD | cut -d. -f2); \
	PATCH=$$(echo $$OLD | cut -d. -f3); \
	if [ -n "$(RELEASE_VERSION)" ]; then \
		NEW="$(RELEASE_VERSION)"; \
	else \
		NEW_PATCH=$$((PATCH + 1)); \
		while git tag -l "v$$MAJOR.$$MINOR.$$NEW_PATCH" | grep -q .; do \
			NEW_PATCH=$$((NEW_PATCH + 1)); \
		done; \
		NEW="$$MAJOR.$$MINOR.$$NEW_PATCH"; \
	fi; \
	if [ "$$(uname)" = "Darwin" ]; then \
		sed -i '' "s/^version = \"$$OLD\"/version = \"$$NEW\"/" Cargo.toml; \
	else \
		sed -i "s/^version = \"$$OLD\"/version = \"$$NEW\"/" Cargo.toml; \
	fi && \
	git add Cargo.toml && git commit -m "chore(release): v$$NEW" && \
	git tag "v$$NEW" && git push && git push --tags

run:
	cargo run --bin qol-tray

dev:
	-pkill -f 'qol-tray'
	cargo run --bin qol-tray --features dev

test:
	cargo test

check:
	cargo check

clean:
	cargo clean

contracts:
	python3 scripts/validate_first_party_plugin_contracts.py --plugins-root ../qol-tools

install:
	cargo build --release --bins
	target/release/qol-tray-install

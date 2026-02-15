.PHONY: build release run dev test check clean contracts install

build:
	cargo build

release:
	@cargo test && \
	OLD=$$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/'); \
	MAJOR=$$(echo $$OLD | cut -d. -f1); \
	MINOR=$$(echo $$OLD | cut -d. -f2); \
	PATCH=$$(echo $$OLD | cut -d. -f3); \
	NEW="$$MAJOR.$$MINOR.$$((PATCH + 1))"; \
	sed -i "s/^version = \"$$OLD\"/version = \"$$NEW\"/" Cargo.toml && \
	cargo build --release && \
	cargo deb --no-build && \
	git add Cargo.toml && git commit -m "chore(release): v$$NEW" && git push && \
	gh release create "v$$NEW" target/debian/*.deb --title "v$$NEW" --generate-notes

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

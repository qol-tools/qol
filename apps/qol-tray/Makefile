.PHONY: run dev test clean install deb release

run:
	cargo run

dev:
	-pkill -f 'qol-tray'
	cargo run --features dev

test:
	cargo test

clean:
	cargo clean

install:
	cargo build --release
	sudo cp target/release/qol-tray /usr/bin/qol-tray

deb:
	cargo build --release
	cargo deb --no-build

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

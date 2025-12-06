.PHONY: test release

test:
	cargo test

release:
	@cargo test && \
	OLD=$$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/'); \
	MAJOR=$$(echo $$OLD | cut -d. -f1); \
	MINOR=$$(echo $$OLD | cut -d. -f2); \
	PATCH=$$(echo $$OLD | cut -d. -f3); \
	NEW="$$MAJOR.$$MINOR.$$((PATCH + 1))"; \
	sed -i "s/^version = \"$$OLD\"/version = \"$$NEW\"/" Cargo.toml && \
	cargo build --release && \
	cp target/release/launcher target/release/launcher-linux-x86_64 && \
	git add Cargo.toml && git commit -m "chore(release): v$$NEW" && git push && \
	gh release create "v$$NEW" target/release/launcher-linux-x86_64 --title "v$$NEW" --generate-notes

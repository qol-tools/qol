.PHONY: test dev release gpui-test gpui-list

GPUI_DIR := gpui-prototype/gpui-test

test:
	cargo test

gpui-test:
	cd $(GPUI_DIR) && cargo test --test property_tests

gpui-list:
	@ls -1 $(GPUI_DIR)/src/bin/*.rs 2>/dev/null | xargs -I {} basename {} .rs | sort

gpui-%:
	cd $(GPUI_DIR) && cargo run --bin $*

dev:
	cargo build --release

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

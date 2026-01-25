.PHONY: test dev release gpui-test gpui-list

GPUI_DIR := gpui-prototype/gpui-test

test:
	cargo test

gpui-test:
	cd $(GPUI_DIR) && cargo test --tests

gpui-list:
	@ls -1 $(GPUI_DIR)/src/bin/*.rs 2>/dev/null | xargs -I {} basename {} .rs | sort

gpui-%:
	@NAME="$*"; \
	if echo "$$NAME" | grep -Eq '^[0-9][0-9]$$'; then \
		BIN="$$(ls -1 $(GPUI_DIR)/src/bin/$$NAME\_*.rs 2>/dev/null | head -1 | xargs -I {} basename {} .rs)"; \
	elif ls -1 $(GPUI_DIR)/src/bin/$$NAME.rs >/dev/null 2>&1; then \
		BIN="$$NAME"; \
	else \
		BIN="$$(ls -1 $(GPUI_DIR)/src/bin/*.rs 2>/dev/null | xargs -I {} basename {} .rs | grep -i "$$NAME" | head -1)"; \
	fi; \
	if [ -z "$$BIN" ]; then \
		echo "No bin matches '$$NAME'"; \
		exit 1; \
	fi; \
	cd $(GPUI_DIR) && cargo run --bin "$$BIN"

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

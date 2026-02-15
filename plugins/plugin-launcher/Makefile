.PHONY: dev release test list

BINARY=launcher
OUTPUT=./$(BINARY)
STAGED=./$(BINARY).new

dev:
	cargo build
	install -m 755 target/debug/$(BINARY) $(STAGED)
	mv -f $(STAGED) $(OUTPUT)

release:
	@cargo test --tests && \
	OLD=$$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/'); \
	MAJOR=$$(echo $$OLD | cut -d. -f1); \
	MINOR=$$(echo $$OLD | cut -d. -f2); \
	PATCH=$$(echo $$OLD | cut -d. -f3); \
	if [ -n "$(RELEASE_VERSION)" ]; then \
		NEW="$(RELEASE_VERSION)"; \
	else \
		NEW="$$MAJOR.$$MINOR.$$((PATCH + 1))"; \
	fi; \
	sed -i "s/^version = \"$$OLD\"/version = \"$$NEW\"/" Cargo.toml && \
	cargo build --release && \
	cp target/release/$(BINARY) target/release/$(BINARY)-linux-x86_64 && \
	git add Cargo.toml && git commit -m "chore(release): v$$NEW" && git push && \
	gh release create "v$$NEW" target/release/$(BINARY)-linux-x86_64 --title "v$$NEW" --generate-notes

test:
	cargo test --tests

list:
	@ls -1 examples/*.rs 2>/dev/null | xargs -I {} basename {} .rs | sort

example-%:
	@NAME="$*"; \
	if ls -1 examples/$$NAME.rs >/dev/null 2>&1; then \
		BIN="$$NAME"; \
	elif echo "$$NAME" | grep -Eq '^[0-9][0-9]$$'; then \
		BIN="$$(ls -1 examples/$$NAME\_*.rs 2>/dev/null | head -1 | xargs -I {} basename {} .rs)"; \
	else \
		BIN="$$(ls -1 examples/*.rs 2>/dev/null | xargs -I {} basename {} .rs | grep -i "$$NAME" | head -1)"; \
	fi; \
	if [ -z "$$BIN" ]; then \
		echo "No example matches '$$NAME'"; \
		exit 1; \
	fi; \
	cargo run --example "$$BIN"

.PHONY: dev release test list

BINARY=launcher
OUTPUT=./$(BINARY)
STAGED=./$(BINARY).new

dev:
	cargo build
	install -m 755 target/debug/$(BINARY) $(STAGED)
	mv -f $(STAGED) $(OUTPUT)

release:
	@set -e; \
	cargo test --tests; \
	git fetch --tags --force; \
	OLD=$$(grep '^[[:space:]]*version[[:space:]]*=' plugin.toml | head -1 | sed 's/^[^"]*"\([^"]*\)".*/\1/'); \
	if [ -z "$$OLD" ]; then \
		echo "Failed to parse version from plugin.toml"; \
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
		sed -i '' "s/version = \"$$OLD\"/version = \"$$NEW\"/" plugin.toml; \
		sed -i '' "s/^version = \"[^\"]*\"/version = \"$$NEW\"/" Cargo.toml; \
	else \
		sed -i "s/version = \"$$OLD\"/version = \"$$NEW\"/" plugin.toml; \
		sed -i "s/^version = \"[^\"]*\"/version = \"$$NEW\"/" Cargo.toml; \
	fi; \
	git add plugin.toml Cargo.toml; \
	git commit -m "chore(release): v$$NEW"; \
	git tag "v$$NEW"; \
	git push; \
	git push --tags

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

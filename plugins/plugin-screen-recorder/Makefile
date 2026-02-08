.PHONY: dev release

BINARY=screen-recorder

dev:
	cargo build
	cp target/debug/$(BINARY) ./$(BINARY)

release:
	cargo build --release
	cp target/release/$(BINARY) ./$(BINARY)

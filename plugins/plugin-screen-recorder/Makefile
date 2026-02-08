.PHONY: dev release

BINARY=screen-recorder
OUTPUT=./$(BINARY)
STAGED=./$(BINARY).new

dev:
	cargo build
	install -m 755 target/debug/$(BINARY) $(STAGED)
	mv -f $(STAGED) $(OUTPUT)

release:
	cargo build --release
	install -m 755 target/release/$(BINARY) $(STAGED)
	mv -f $(STAGED) $(OUTPUT)

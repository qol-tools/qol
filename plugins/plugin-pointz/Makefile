.PHONY: dev build release run check test clean

BINARY=pointzerver
OUTPUT=./$(BINARY)
STAGED=./$(BINARY).new

dev:
	cargo build
	install -m 755 target/debug/$(BINARY) $(STAGED)
	mv -f $(STAGED) $(OUTPUT)

build: dev

release:
	cargo build --release
	install -m 755 target/release/$(BINARY) $(STAGED)
	mv -f $(STAGED) $(OUTPUT)

run:
	RUST_LOG=info cargo run

check:
	cargo check

test:
	cargo test

clean:
	cargo clean
	rm -f $(OUTPUT) $(STAGED)

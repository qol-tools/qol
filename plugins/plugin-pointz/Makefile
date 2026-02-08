.PHONY: dev build release run check test clean

BINARY=pointzerver

dev:
	cargo build
	cp target/debug/$(BINARY) ./$(BINARY)

build: dev

release:
	cargo build --release
	cp target/release/$(BINARY) ./$(BINARY)

run:
	RUST_LOG=info cargo run

check:
	cargo check

test:
	cargo test

clean:
	cargo clean
	rm -f ./$(BINARY)

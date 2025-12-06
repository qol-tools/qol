.PHONY: run dev test clean install

run:
	cargo run

dev:
	cargo run --features dev

test:
	cargo test

clean:
	cargo clean

install:
	cargo build --release
	sudo cp target/release/qol-tray /usr/bin/qol-tray

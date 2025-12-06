.PHONY: run dev test clean install deb release

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

deb:
	cargo build --release
	cargo deb --no-build

release:
	@VERSION=$$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/'); \
	cargo build --release && \
	cargo deb --no-build && \
	gh release create "v$$VERSION" target/debian/*.deb --title "v$$VERSION" --generate-notes

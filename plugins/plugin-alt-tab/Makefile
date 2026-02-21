PLUGIN_NAME = alt-tab
BINARY_NAME = alt-tab

# Get current architecture and OS
ARCH ?= $(shell uname -m | sed 's/x86_64/amd64/' | sed 's/aarch64/arm64/')
OS ?= $(shell uname -s | tr '[:upper:]' '[:lower:]')

# Maps 'amd64' to 'x86_64' for Cargo target triples
CARGO_ARCH_amd64 = x86_64
CARGO_ARCH_arm64 = aarch64
CARGO_ARCH = $(CARGO_ARCH_$(ARCH))

# Determine standard cargo target based on OS and architecture
ifeq ($(OS),linux)
    CARGO_TARGET = $(CARGO_ARCH)-unknown-linux-gnu
else ifeq ($(OS),darwin)
    CARGO_TARGET = $(CARGO_ARCH)-apple-darwin
else
    $(error Unsupported OS: $(OS))
endif

.PHONY: all build clean install install-dev release test

all: build

build:
	cargo build --release

test:
	cargo test

install: build
	mkdir -p ~/.config/qol-tray/plugins/$(PLUGIN_NAME)
	cp plugin.toml ~/.config/qol-tray/plugins/$(PLUGIN_NAME)/
	cp target/release/$(BINARY_NAME) ~/.config/qol-tray/plugins/$(PLUGIN_NAME)/$(BINARY_NAME)-$(OS)-$(ARCH)

install-dev:
	cargo build
	mkdir -p ~/.config/qol-tray/plugins/$(PLUGIN_NAME)
	cp plugin.toml ~/.config/qol-tray/plugins/$(PLUGIN_NAME)/
	cp target/debug/$(BINARY_NAME) ~/.config/qol-tray/plugins/$(PLUGIN_NAME)/$(BINARY_NAME)-$(OS)-$(ARCH)

clean:
	cargo clean

release:
	cargo build --release --target $(CARGO_TARGET)
	mkdir -p dist
	cp target/$(CARGO_TARGET)/release/$(BINARY_NAME) dist/$(BINARY_NAME)-$(OS)-$(ARCH)

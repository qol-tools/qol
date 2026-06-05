.PHONY: dev release test trace

BINARY=alt-tab
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

test:
	cargo test --tests

trace:
	python3 tools/compact_trace.py

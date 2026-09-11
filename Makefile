.PHONY: build test release clean check fmt clippy

build:
	cargo build --workspace

test:
	cargo test --workspace

release:
	cargo build --release --workspace

clean:
	cargo clean

check:
	cargo check --workspace

fmt:
	cargo fmt --all

clippy:
	cargo clippy --workspace -- -D warnings

## Convenience targets

trace:
	cargo run --bin patternstudio -- trace $(INPUT) -o $(OUTPUT) $(FLAGS)

pattern:
	cargo run --bin patternstudio -- pattern -k $(KIND) -n $(N) -o $(OUTPUT)

server:
	cargo run --bin patternstudio-server
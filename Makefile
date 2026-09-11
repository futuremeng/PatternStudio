.PHONY: build test release clean check fmt clippy web web-dev web-serve desktop desktop-dev desktop-run

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

## Web UI

web:
	cd web && npm install && npm run build

web-dev:
	cd web && npm run dev

web-serve:
	cargo run --bin patternstudio-server

## Desktop (Tauri)

desktop:
	cd web && npm install && npm run build
	cargo build --manifest-path desktop/Cargo.toml

desktop-dev:
	cd desktop && npm install && npx tauri dev

desktop-run:
	cargo run --manifest-path desktop/Cargo.toml --features tauri/custom-protocol
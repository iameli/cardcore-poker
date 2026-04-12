# Build the library
build:
    cargo build

# Build in release mode
build-release:
    cargo build --release

# Build WASM package
build-wasm:
    wasm-pack build --target web --release

# Run native tests
test:
    cargo test --release

# Run WASM browser tests in headless Chrome
test-wasm:
    wasm-pack test --headless --chrome --release

# Run Playwright browser tests for the web frontend
test-web: build-wasm
    pnpm install
    pnpm test

# Run the dev server
dev: build-wasm
    pnpm install
    pnpm exec vite

# Run all tests (native + WASM + web)
test-all: test test-wasm test-web

# Run the text-based CLI
play:
    cargo run --bin poker

# Regenerate lexicon types from JSON schemas
lexgen:
    rm -rf src/lexicon
    mkdir -p src/lexicon
    jacquard-codegen -i lexicons -o src/lexicon
    mv src/lexicon/lib.rs src/lexicon/mod.rs
    sed -i 's/^#\[cfg(feature = "re_cardco")\]//' src/lexicon/mod.rs
    find src/lexicon -name "*.rs" -exec sed -i 's/use crate::builder_types/use crate::lexicon::builder_types/g' {} +
    find src/lexicon -name "*.rs" -exec sed -i 's/use alloc::collections::BTreeMap/use std::collections::BTreeMap/g' {} +
    find src/lexicon -name "*.rs" -exec sed -i 's/alloc::collections::BTreeMap/std::collections::BTreeMap/g' {} +
    find src/lexicon -name "*.rs" -exec sed -i 's|use crate::re_cardco|use crate::lexicon::re_cardco|g' {} +

# Build the Docker image for CI
docker-build:
    docker build -t cardcore-poker-test .

# Run all tests in Docker
docker-test:
    docker run --rm cardcore-poker-test

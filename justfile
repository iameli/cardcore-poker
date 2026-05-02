# Hostname override for `just build-web`. Bakes into the OAuth client_id and
# the generated oauth-client-metadata.json so a build can be served behind
# an HTTPS reverse proxy on a different host. Override at invocation time:
#   just HOSTNAME=art.iameli.link build-web
#   HOSTNAME=art.iameli.link just build-web
HOSTNAME := env_var_or_default("HOSTNAME", "")

# Build the library
build:
    cargo build

# Build the web app from a cold checkout. Produces packages/web/dist/.
# Set HOSTNAME to swap the OAuth host away from cardco.re (see comment above).
build-web: build-wasm
    pnpm install --frozen-lockfile
    OAUTH_HOST="{{HOSTNAME}}" pnpm --filter @cardcore/web build

# Build in release mode
build-release:
    cargo build --release

# Build WASM package
build-wasm:
    wasm-pack build --target web --release

# Build WASM for Node.js (multiplayer test)
build-wasm-node:
    wasm-pack build --target nodejs --release --out-dir pkg-node

# Run native tests
test:
    cargo test --release

# Run WASM browser tests in headless Chrome
test-wasm:
    wasm-pack test --headless --chrome --release

# Run Playwright browser tests for the web frontend
test-web: build-wasm
    pnpm install
    pnpm --filter @cardcore/web test

# Run the dev server (Svelte client on :5173 + WS room server on :3003)
dev: build-wasm
    pnpm install
    pnpm --filter @cardcore/web dev:all

# Run all tests (native + WASM + web)
test-all: test test-wasm test-web

# Run the text-based CLI
play:
    cargo run --bin poker

# Run multiplayer test (requires a running PDS)
test-multiplayer: build-wasm-node
    pnpm install
    pnpm run test:multiplayer

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

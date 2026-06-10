FROM rust:bookworm

# Chrome + chromedriver for WASM browser tests
RUN apt-get update && apt-get install -y \
    wget \
    gnupg \
    unzip \
    && wget -q -O - https://dl.google.com/linux/linux_signing_key.pub | apt-key add - \
    && echo "deb [arch=amd64] http://dl.google.com/linux/chrome/deb/ stable main" > /etc/apt/sources.list.d/google-chrome.list \
    && apt-get update && apt-get install -y google-chrome-stable \
    && rm -rf /var/lib/apt/lists/*

# Install matching chromedriver
RUN CHROME_VERSION=$(google-chrome --version | grep -oP '\d+\.\d+\.\d+\.\d+') \
    && wget -q "https://storage.googleapis.com/chrome-for-testing-public/${CHROME_VERSION}/linux64/chromedriver-linux64.zip" -O /tmp/chromedriver.zip \
    && unzip /tmp/chromedriver.zip -d /tmp \
    && mv /tmp/chromedriver-linux64/chromedriver /usr/local/bin/ \
    && chmod +x /usr/local/bin/chromedriver \
    && rm -rf /tmp/chromedriver* \
    && chromedriver --version

# Node.js + pnpm for web frontend, multiplayer test, and dev-env
RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
    && apt-get install -y nodejs \
    && npm install -g pnpm@9 \
    && node --version && pnpm --version

# WASM target + wasm-pack
RUN rustup target add wasm32-unknown-unknown \
    && cargo install wasm-pack

WORKDIR /app

# Install JS deps from the manifests alone, so this layer (and the Playwright
# browser download below) stays cached across source-only changes. Lifecycle
# scripts are skipped — dev-env's prepare needs sources that aren't copied
# yet; the full install below runs them.
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY packages/web/package.json packages/web/
COPY packages/dev-env/package.json packages/dev-env/
RUN pnpm install --frozen-lockfile --ignore-scripts

# Playwright browser for web frontend tests. Must run AFTER pnpm install and
# through the workspace's own playwright, so the downloaded browser revision
# matches the version pinned in the lockfile — a bare `npx playwright install`
# resolves whatever's latest and drifts out of sync with the pin.
RUN pnpm --filter @cardcore/web exec playwright install --with-deps chromium

COPY . .

# Re-link JS deps for the full tree and pre-fetch Rust deps. The manifests-only
# install above ran with --ignore-scripts, and pnpm does NOT re-run dependency
# build scripts on a later install that finds node_modules up to date — so
# rebuild them explicitly (better-sqlite3's native binding is what the dev PDS
# boots on; without this the whole Playwright stage dies).
RUN pnpm install --frozen-lockfile && pnpm rebuild -r && cargo fetch

# Build WASM (both web and node targets)
RUN wasm-pack build --target web --release \
    && wasm-pack build --target nodejs --release --out-dir pkg-node

# Run all tests: native Rust + WASM browser + web frontend (Playwright over
# the local PDS) + 4-player AT-Protocol-firehose multiplayer test
CMD ["sh", "-c", "\
    echo '=== Native Rust tests ===' && cargo test --release && \
    echo '=== WASM browser tests ===' && wasm-pack test --headless --chrome --release && \
    echo '=== Web frontend tests (Playwright + local PDS) ===' && pnpm --filter @cardcore/web test && \
    echo '=== Multiplayer integration (firehose) ===' && npx tsx multiplayer-test.ts \
"]

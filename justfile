# rosso task runner. `just` with no args lists recipes.
#
# Yarn = the repo-vendored release pinned by `yarnPath` in frontend/.yarnrc.yml,
# run via node. No global yarn / corepack needed (recipes run under sh, which
# can't see a shell yarn function), and it auto-tracks `yarn set version` bumps.

default:
    @just --list

# Install frontend deps.
install:
    cd frontend && node .yarn/releases/yarn-*.cjs install

# Backend (bacon, :3008) + frontend (Vite, :5173) together, one Ctrl-C stops
# both. The SPA proxies /api and /status to the backend; bacon hot-reloads the
# Rust on src + .env changes. Backend runs headless so the logs compose into one
# stream — run plain `bacon` in backend/ when you want the TUI.
dev:
    #!/usr/bin/env bash
    set -euo pipefail
    # Tear down every child and its grandchildren (the binary under bacon, vite
    # under yarn) so nothing orphans and holds its port. Killing only the
    # children — NOT `kill 0` — leaves `just` and the shell unsignalled.
    pids=""
    cleanup() {
        trap - INT TERM EXIT
        for p in $pids; do
            pkill -P "$p" 2>/dev/null || true
            kill "$p" 2>/dev/null || true
        done
    }
    trap cleanup INT TERM EXIT
    ( cd backend && exec bacon --headless -j run ) &
    pids="$pids $!"
    ( cd frontend && exec node .yarn/releases/yarn-*.cjs dev ) &
    pids="$pids $!"
    wait

# Build: frontend, then the rust workspace.
build:
    cd frontend && node .yarn/releases/yarn-*.cjs build
    cargo build --release --workspace

# Lint across the repo.
lint:
    cd frontend && node .yarn/releases/yarn-*.cjs lint
    cargo clippy --workspace --all-targets -- -D warnings

# Formatting check. Apply with `yarn format:fix` / `cargo fmt --all`.
format:
    cd frontend && node .yarn/releases/yarn-*.cjs format
    cargo fmt --all -- --check

# Unit tests (fast).
test:
    cargo test --workspace
    cd frontend && node .yarn/releases/yarn-*.cjs test

# Integration tests — spawn the real binary, so they bind ports and are #[ignore]
# by default. The explicit build is load-bearing: `cargo test -p
# rosso-integration` does NOT rebuild another package's binary, so without it the
# suite silently exercises whatever stale backend is sitting in target/.
test-integration:
    cargo build -p rosso-backend
    cargo test -p rosso-integration -- --ignored

# syntax=docker/dockerfile:1
#
#   docker build --target rosso -t rosso .
#
# The Rust backend cross-compiles (tonistiigi/xx) to a static musl binary on
# scratch and ships the built SPA beside it. The node-alpine tag must match
# frontend/.node-version.

# --- Cross-compilation helper ---
FROM --platform=$BUILDPLATFORM tonistiigi/xx AS xx

# ============================================================================
# Frontend (vendored yarn — no corepack)
# ============================================================================
# The yarn binary is committed at frontend/.yarn/releases and pinned by
# .yarnrc.yml's `yarnPath`, so the build doesn't depend on the base image's
# bundled yarn (node 25+ dropped the corepack bundle). Manifests are copied first
# so `install` caches across source-only changes.
FROM --platform=$BUILDPLATFORM node:26-alpine AS frontend-build
WORKDIR /app
COPY frontend/package.json frontend/yarn.lock frontend/.yarnrc.yml ./
COPY frontend/.yarn/releases ./.yarn/releases
RUN node .yarn/releases/yarn-*.cjs install --immutable --network-timeout 1000000
COPY frontend ./
RUN node .yarn/releases/yarn-*.cjs build

# ============================================================================
# Rust backend (warm dep cache via stub source)
# ============================================================================
FROM --platform=$BUILDPLATFORM rust:1-alpine AS workspace-deps
COPY --from=xx / /
RUN apk add --no-cache clang lld musl-dev curl
ARG TARGETPLATFORM
RUN xx-apk add --no-cache musl-dev gcc
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY backend/Cargo.toml backend/Cargo.toml
COPY integration/Cargo.toml integration/Cargo.toml
# Stub sources so cargo can parse the workspace and warm the dep cache before the
# real source lands. The integration crate is a workspace member, so it needs one
# too even though the image never builds it.
RUN mkdir -p backend/src integration/src \
    && printf 'fn main() {}\n' > backend/src/main.rs \
    && printf '\n' > backend/src/lib.rs \
    && printf '\n' > integration/src/lib.rs \
    && xx-cargo build --release -p rosso-backend

FROM workspace-deps AS backend-build
ARG TARGETPLATFORM
COPY backend/src ./backend/src
# `touch` so cargo notices the stub→real source swap and rebuilds the package.
RUN touch backend/src/main.rs backend/src/lib.rs \
    && xx-cargo build --release -p rosso-backend \
    && cp target/*/release/rosso-backend /rosso-backend

# ============================================================================
# Runtime image (scratch + binary + dist + certs)
# ============================================================================
FROM scratch AS rosso
WORKDIR /app
LABEL org.opencontainers.image.description="rosso — a feed reader that fetches in the background and ranks what it finds"
COPY --from=backend-build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --from=backend-build /rosso-backend ./rosso-backend
COPY --from=frontend-build /app/dist ./dist
ENV STATIC_DIR=./dist
ENV ROSSO_DB_PATH=/data/rosso.db
ENV ROSSO_BIND=0.0.0.0:3008
USER 1000
EXPOSE 3008
CMD ["./rosso-backend"]

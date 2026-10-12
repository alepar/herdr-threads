# Linux e2e runner image for herdr-threads (scripts/linux-e2e/run builds it; see README.md there).
#
# ubuntu 24.04, native linux/arm64 or linux/amd64 (TARGETARCH). Every download is pinned and
# checksum-verified for the build architecture:
#   - Rust via rustup-init (RUSTUP_VERSION) with the toolchain CI and release.yml pin (RUST_TOOLCHAIN),
#     plus the <arch>-unknown-linux-musl target and musl-tools for the static release build;
#   - Herdr from tests/herdr-releases.tsv (HERDR_VERSION, default the newest listed release);
#   - Node 22 from the official tarball (NODE_VERSION + per-arch SHA-256);
#   - Claude Code and the Codex CLI from npm at exact versions, the top-level package tarball checked
#     against its pinned npm integrity (their per-platform binaries are exact-version optional
#     dependencies, verified by npm against the registry's integrity). Installed, never logged in:
#     the image holds no credentials, and live runs provision them per container (README.md).
# python3, git, procps and lsof serve the e2e scripts (scripts/check-no-leaked-processes needs lsof).
# Runs as the non-root user `tester` (no sudo). Containers must run with `docker run --init`.
#
# Build context: `git archive HEAD scripts/linux-e2e tests/herdr-releases.tsv` (the run script pipes it).
FROM ubuntu:24.04

ARG TARGETARCH
ARG RUST_TOOLCHAIN=1.94.0
ARG RUSTUP_VERSION=1.29.1
ARG RUSTUP_SHA256_ARM64=15f6e4ce9f583b929c996c91562bad6d4454f3281de858b02cdfdef615fac433
ARG RUSTUP_SHA256_AMD64=dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71
ARG NODE_VERSION=22.23.3
ARG NODE_SHA256_ARM64=a44aeb94849a299b22df10b9e622ec2f605c2183501bc40590705131de7c740f
ARG NODE_SHA256_AMD64=df450af89261115ef9f9e3830c3eeb2cc9213b63c720b1af623cb5dcbe2e02de
ARG CLAUDE_CODE_VERSION=2.1.296
ARG CLAUDE_CODE_INTEGRITY=sha512-OX/k/rpcthMqnFKOWcpNcbiLsMKtAjLOpQNJHlYiDCFUznWCIwdRTyl/p9SdHVrsrzZuPMCNqZjnpakIni6upw==
ARG CODEX_VERSION=0.162.1
ARG CODEX_INTEGRITY=sha512-NWZdi/kxyjv/8EUGFupziGU38YyleugZRM4JXgY5XFH7FUmaFA33NZS2Bmq0HPazf7S3jJQQWsZ/jAK9jsrV3Q==
ARG HERDR_VERSION=

LABEL dev.herdr-threads.linux-e2e=1

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential musl-tools file curl ca-certificates git python3 procps lsof \
      pkg-config less locales xz-utils \
    && rm -rf /var/lib/apt/lists/* && locale-gen en_US.UTF-8

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

# Architecture names: Docker's TARGETARCH (arm64/amd64) -> Herdr/Rust (aarch64/x86_64), Node (arm64/x64).
RUN case "${TARGETARCH:-$(dpkg --print-architecture)}" in \
      arm64) echo "aarch64 arm64 $RUSTUP_SHA256_ARM64 $NODE_SHA256_ARM64" ;; \
      amd64) echo "x86_64 x64 $RUSTUP_SHA256_AMD64 $NODE_SHA256_AMD64" ;; \
      *) echo "unsupported architecture: ${TARGETARCH}" >&2; exit 1 ;; \
    esac > /etc/ht-arch && cat /etc/ht-arch

# Herdr: the official release binary for this architecture, checked against tests/herdr-releases.tsv.
COPY tests/herdr-releases.tsv /opt/ht/herdr-releases.tsv
RUN read -r arch _ _ _ < /etc/ht-arch; \
    version=${HERDR_VERSION:-$(awk -F'\t' '!/^#/ && NF == 3 { print $1 }' /opt/ht/herdr-releases.tsv | sort -V | tail -n 1)}; \
    digest=$(awk -F'\t' -v v="$version" -v a="herdr-linux-$arch" '$1 == v && $2 == a { print $3 }' /opt/ht/herdr-releases.tsv); \
    test -n "$digest" || { echo "no herdr-linux-$arch $version in herdr-releases.tsv" >&2; exit 1; }; \
    curl -fsSL --retry 3 -o /usr/local/bin/herdr "https://github.com/herdrdev/herdr/releases/download/v$version/herdr-linux-$arch"; \
    echo "$digest  /usr/local/bin/herdr" | sha256sum -c -; \
    chmod 755 /usr/local/bin/herdr; \
    test "$(herdr --version)" = "herdr $version"; \
    echo "$version" > /opt/ht/herdr-version

# Node 22: official tarball, checksum-pinned per architecture.
RUN read -r _ node_arch _ node_sha < /etc/ht-arch; \
    tarball="node-v${NODE_VERSION}-linux-${node_arch}.tar.xz"; \
    curl -fsSL --retry 3 -o "/tmp/$tarball" "https://nodejs.org/dist/v${NODE_VERSION}/$tarball"; \
    echo "$node_sha  /tmp/$tarball" | sha256sum -c -; \
    mkdir -p /opt/node && tar -xJf "/tmp/$tarball" -C /opt/node --strip-components=1 && rm "/tmp/$tarball"; \
    /opt/node/bin/node --version

# Claude Code and Codex: exact versions, top-level tarball integrity pinned. No login.
ENV PATH=/opt/node/bin:$PATH
RUN cd /tmp && for spec in "@anthropic-ai/claude-code@$CLAUDE_CODE_VERSION $CLAUDE_CODE_INTEGRITY" \
                         "@openai/codex@$CODEX_VERSION $CODEX_INTEGRITY"; do \
      read -r pkg integrity <<< "$spec"; \
      tgz=$(npm pack --silent "$pkg"); \
      actual="sha512-$(openssl dgst -sha512 -binary "$tgz" | base64 -w0)"; \
      test "$actual" = "$integrity" || { echo "integrity mismatch for $pkg: $actual" >&2; exit 1; }; \
      npm install -g --no-fund --no-audit "./$tgz" && rm "$tgz"; \
    done; \
    npm cache clean --force; \
    claude --version && codex --version

# The non-root user and its cached build directories (named volumes mount here when the run script
# is given --cache; their first mount copies this ownership).
RUN useradd -m -s /bin/bash tester
USER tester
ENV HOME=/home/tester LANG=en_US.UTF-8 \
    PATH=/home/tester/.cargo/bin:/home/tester/.local/bin:/opt/node/bin:/usr/local/bin:/usr/bin:/bin \
    DISABLE_AUTOUPDATER=1
RUN read -r arch _ rustup_sha _ < /etc/ht-arch; \
    curl -fsSL --retry 3 -o /tmp/rustup-init \
      "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/${arch}-unknown-linux-gnu/rustup-init"; \
    echo "$rustup_sha  /tmp/rustup-init" | sha256sum -c -; \
    chmod 755 /tmp/rustup-init && /tmp/rustup-init -y --no-modify-path --profile minimal \
      --default-toolchain "$RUST_TOOLCHAIN" --target "${arch}-unknown-linux-musl" && rm /tmp/rustup-init; \
    rustc --version; \
    mkdir -p /home/tester/.cargo/registry /home/tester/src/target /home/tester/.cache/herdr-releases \
             /home/tester/evidence
WORKDIR /home/tester

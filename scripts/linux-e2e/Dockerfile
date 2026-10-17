# Linux e2e runner image for herdr-threads (scripts/linux-e2e/run builds it; see README.md there).
#
# ubuntu 24.04, native linux/arm64 or linux/amd64 (TARGETARCH). Every download is pinned and
# checksum-verified for the build architecture:
#   - Rust via rustup-init (RUSTUP_VERSION) with the toolchain CI and release.yml pin (RUST_TOOLCHAIN),
#     plus the <arch>-unknown-linux-musl target and musl-tools for the static release build;
#   - Herdr from tests/herdr-releases.tsv (HERDR_VERSION, default the newest listed release);
#   - Node 22 from the official tarball (NODE_VERSION + per-arch SHA-256);
#   - Claude Code and the Codex CLI with `npm ci` from npm/package-lock.json, which pins the integrity
#     (sha512) of every tarball, the per-platform native packages included (they run with the
#     credentials). Installed, never logged in: the image holds no credentials, and live runs provision
#     them per container (README.md). Bump them with `npm install --package-lock-only` in npm/.
#   - the base image by its multi-arch index digest (ubuntu:24.04 at the time of pinning).
# python3, git, procps and lsof serve the e2e scripts (scripts/check-no-leaked-processes needs lsof).
# openssh-client/-server serve `run real-ssh` (real-ssh-e2e.py starts sshd as root in a container of this image,
# key-only, as the sshd stage of the two-node check; nothing starts it otherwise).
# Runs as the non-root user `tester` (no sudo). Containers must run with `docker run --init`.
#
# Build context: `git archive <sha> scripts/linux-e2e tests/herdr-releases.tsv` (the run script extracts it).
FROM ubuntu:24.04@sha256:534baea6a22c03a63003dbc8dbe78fe34bc0d7e595d9a9dc9834884ff530eb55

ARG TARGETARCH
ARG RUST_TOOLCHAIN=1.94.0
ARG RUSTUP_VERSION=1.29.1
ARG RUSTUP_SHA256_ARM64=15f6e4ce9f583b929c996c91562bad6d4454f3281de858b02cdfdef615fac433
ARG RUSTUP_SHA256_AMD64=dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71
ARG NODE_VERSION=22.23.3
ARG NODE_SHA256_ARM64=a44aeb94849a299b22df10b9e622ec2f605c2183501bc40590705131de7c740f
ARG NODE_SHA256_AMD64=df450af89261115ef9f9e3830c3eeb2cc9213b63c720b1af623cb5dcbe2e02de
ARG HERDR_VERSION=

LABEL dev.herdr-threads.linux-e2e=1

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential musl-tools file curl ca-certificates git python3 procps lsof \
      pkg-config less locales xz-utils openssh-client openssh-server \
    && rm -rf /var/lib/apt/lists/* && locale-gen en_US.UTF-8

# errexit + nounset + pipefail: every command in a RUN is fatal, so no later command can mask a failure.
SHELL ["/bin/bash", "-euo", "pipefail", "-c"]

# Architecture names: Docker's TARGETARCH (arm64/amd64) -> Herdr/Rust (aarch64/x86_64), Node (arm64/x64).
RUN case "${TARGETARCH:-$(dpkg --print-architecture)}" in \
      arm64) echo "aarch64 arm64 $RUSTUP_SHA256_ARM64 $NODE_SHA256_ARM64" ;; \
      amd64) echo "x86_64 x64 $RUSTUP_SHA256_AMD64 $NODE_SHA256_AMD64" ;; \
      *) echo "unsupported architecture: ${TARGETARCH}" >&2; exit 1 ;; \
    esac > /etc/ht-arch && cat /etc/ht-arch

# Downloads go through image-fetch.sh: download -> SHA-256 -> version, each fatal, promoted only when all pass.
COPY scripts/linux-e2e/image-fetch.sh /opt/ht/image-fetch.sh

# Herdr: the official release binary for this architecture, checked against tests/herdr-releases.tsv.
COPY tests/herdr-releases.tsv /opt/ht/herdr-releases.tsv
RUN read -r arch _ _ _ < /etc/ht-arch \
    && /opt/ht/image-fetch.sh herdr /opt/ht/herdr-releases.tsv "$arch" /usr/local/bin/herdr /opt/ht/herdr-version \
         "$HERDR_VERSION"

# Node 22: official tarball, checksum-pinned per architecture.
RUN read -r _ node_arch _ node_sha < /etc/ht-arch \
    && /opt/ht/image-fetch.sh node "$NODE_VERSION" "$node_arch" "$node_sha" /opt/node

# Claude Code and Codex: `npm ci` verifies every tarball against the lockfile's integrity. No login.
# --ignore-scripts: no package lifecycle script runs. The only one in this tree is Claude Code's postinstall
# (`node install.cjs`, reviewed for 2.1.296): it copies the binary of the already-installed, lockfile-verified
# native package (@anthropic-ai/claude-code-linux-<arch>) over the bin/claude.exe stub, with no network access
# (its only subprocess is `sysctl` on macOS). It runs explicitly below, under --network=none; Codex 0.162.1
# and both native packages declare no scripts. Re-check this when bumping the lockfile.
ENV PATH=/opt/node/bin:$PATH
COPY scripts/linux-e2e/npm/package.json scripts/linux-e2e/npm/package-lock.json /opt/harnesses/
RUN cd /opt/harnesses && npm ci --ignore-scripts --no-audit --no-fund && npm cache clean --force
# The postinstall without a network (structurally no fetch), then proof that bin/claude.exe is exactly the
# lockfile-verified native package's binary.
RUN --network=none read -r _ node_arch _ _ < /etc/ht-arch \
    && cd /opt/harnesses/node_modules/@anthropic-ai/claude-code && node install.cjs \
    && cmp bin/claude.exe "../claude-code-linux-$node_arch/claude"
RUN cd /opt/harnesses && mkdir bin && ln -s ../node_modules/.bin/claude bin/claude \
    && ln -s ../node_modules/.bin/codex bin/codex && bin/claude --version && bin/codex --version

# The non-root user and its cached build directories (named volumes mount here when the run script
# is given --cache; their first mount copies this ownership).
RUN useradd -m -s /bin/bash tester
USER tester
ENV HOME=/home/tester LANG=en_US.UTF-8 \
    PATH=/home/tester/.cargo/bin:/home/tester/.local/bin:/opt/harnesses/bin:/opt/node/bin:/usr/local/bin:/usr/bin:/bin \
    DISABLE_AUTOUPDATER=1
RUN read -r arch _ rustup_sha _ < /etc/ht-arch \
    && /opt/ht/image-fetch.sh rustup "$RUSTUP_VERSION" "$arch" "$rustup_sha" "$RUST_TOOLCHAIN" "$arch-unknown-linux-musl" \
    && mkdir -p /home/tester/.cargo/registry /home/tester/src/target /home/tester/.cache/herdr-releases \
                /home/tester/evidence
WORKDIR /home/tester

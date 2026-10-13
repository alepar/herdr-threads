#!/usr/bin/env bash
# Pinned downloads for scripts/linux-e2e/Dockerfile: download -> SHA-256 -> version, in that order, each step
# fatal, into a temporary path that is promoted (moved into place, stamped) only after every check passed.
# A failed download, checksum or version check leaves nothing at the destination and exits non-zero.
#
#   image-fetch.sh herdr  TSV ARCH DEST STAMP [VERSION]   # Herdr from tests/herdr-releases.tsv (default: newest)
#   image-fetch.sh node   VERSION NODE_ARCH SHA256 DEST    # official Node tarball, extracted to DEST
#   image-fetch.sh rustup VERSION ARCH SHA256 TOOLCHAIN MUSL_TARGET  # rustup-init, then the pinned toolchain
#
# scripts/tests/test_linux_e2e_run.py runs it with fake curl/sha256sum on PATH (negative controls).
set -euo pipefail

die() { printf 'image-fetch: %s\n' "$*" >&2; exit 1; }
work=''  # the scratch dir; global so the EXIT trap still sees it after the function returned
trap 'if [ -n "$work" ]; then rm -rf "$work"; fi' EXIT

fetch() { # URL SHA256 OUT: download into OUT only if its digest matches
    local url=$1 sha=$2 out=$3 part
    part=$(mktemp "$out.part.XXXXXX")
    if ! curl -fsSL --retry 3 -o "$part" "$url"; then rm -f "$part"; die "download failed: $url"; fi
    if ! printf '%s  %s\n' "$sha" "$part" | sha256sum -c - >&2; then rm -f "$part"; die "checksum mismatch: $url"; fi
    mv -f "$part" "$out"
}

fetch_herdr() { # TSV ARCH DEST STAMP [VERSION]
    local tsv=$1 arch=$2 dest=$3 stamp=$4 version=${5:-} digest reported
    [ -n "$version" ] || version=$(awk -F'\t' '!/^#/ && NF == 3 { print $1 }' "$tsv" | sort -V | tail -n 1)
    digest=$(awk -F'\t' -v v="$version" -v a="herdr-linux-$arch" '$1 == v && $2 == a { print $3 }' "$tsv")
    [ -n "$digest" ] || die "no herdr-linux-$arch $version in $tsv"
    work=$(mktemp -d "$(dirname "$dest")/.herdr.XXXXXX")
    fetch "https://github.com/herdrdev/herdr/releases/download/v$version/herdr-linux-$arch" "$digest" "$work/herdr"
    chmod 755 "$work/herdr"
    reported=$("$work/herdr" --version) || die "herdr --version failed"
    [ "$reported" = "herdr $version" ] || die "herdr reports '$reported', expected 'herdr $version'"
    mv -f "$work/herdr" "$dest"
    printf '%s\n' "$version" > "$stamp"
    printf 'herdr %s -> %s\n' "$version" "$dest"
}

fetch_node() { # VERSION NODE_ARCH SHA256 DEST
    local version=$1 arch=$2 sha=$3 dest=$4 tarball reported
    [ ! -e "$dest" ] || die "$dest already exists"
    tarball="node-v$version-linux-$arch.tar.xz"
    work=$(mktemp -d "$(dirname "$dest")/.node.XXXXXX")
    fetch "https://nodejs.org/dist/v$version/$tarball" "$sha" "$work/$tarball"
    mkdir "$work/root"
    tar -xJf "$work/$tarball" -C "$work/root" --strip-components=1
    reported=$("$work/root/bin/node" --version) || die "node --version failed"
    [ "$reported" = "v$version" ] || die "node reports '$reported', expected 'v$version'"
    mv "$work/root" "$dest"
    printf 'node %s -> %s\n' "$version" "$dest"
}

fetch_rustup() { # VERSION ARCH SHA256 TOOLCHAIN MUSL_TARGET
    local version=$1 arch=$2 sha=$3 toolchain=$4 target=$5 reported
    work=$(mktemp -d)
    fetch "https://static.rust-lang.org/rustup/archive/$version/$arch-unknown-linux-gnu/rustup-init" "$sha" \
        "$work/rustup-init"
    chmod 755 "$work/rustup-init"
    "$work/rustup-init" -y --no-modify-path --profile minimal --default-toolchain "$toolchain" --target "$target"
    reported=$("$HOME/.cargo/bin/rustc" --version) || die "rustc --version failed"
    case "$reported" in "rustc $toolchain "*) ;; *) die "rustc reports '$reported', expected $toolchain" ;; esac
    printf '%s\n' "$reported"
}

[ $# -ge 1 ] || die "usage: image-fetch.sh herdr|node|rustup ARGS..."
what=$1; shift
case "$what" in
    herdr) [ $# -ge 4 ] || die "usage: herdr TSV ARCH DEST STAMP [VERSION]"; fetch_herdr "$@" ;;
    node) [ $# -eq 4 ] || die "usage: node VERSION NODE_ARCH SHA256 DEST"; fetch_node "$@" ;;
    rustup) [ $# -eq 5 ] || die "usage: rustup VERSION ARCH SHA256 TOOLCHAIN MUSL_TARGET"; fetch_rustup "$@" ;;
    *) die "unknown download: $what" ;;
esac

#!/bin/sh
# Package a prebuilt herdr-threads executable as a release archive.
#
# usage: scripts/package-release.sh --binary PATH --os macos|linux
#            --arch aarch64|x86_64 --out DIR [--version VERSION]
#            [--third-party FILE]
#
# Writes DIR/herdr-threads-OS-ARCH.tar.gz with one top-level directory,
# herdr-threads/, holding what the Herdr plugin needs at run time:
# herdr-plugin.toml, scripts/view.sh, scripts/build.sh, bin/herdr-threads,
# the README files, LICENSE-MIT, LICENSE-APACHE, THIRD_PARTY_LICENSES.html (the
# cargo-about output FILE, default THIRD_PARTY_LICENSES.html in the repository
# root; required), VERSION, TARGET and the PREBUILT marker that makes the
# manifest build command keep the shipped executable. VERSION defaults to the
# Cargo.toml package version. Used by .github/workflows/release.yml and by the
# offline installer test (tests/release/install_test.sh).
set -eu

repo_root=$(CDPATH='' cd "$(dirname "$0")/.." && pwd -P)
binary='' os='' arch='' out='' version='' third=''
while [ $# -gt 0 ]; do
    case "$1" in
        --binary) binary=$2; shift 2 ;;
        --os) os=$2; shift 2 ;;
        --arch) arch=$2; shift 2 ;;
        --out) out=$2; shift 2 ;;
        --version) version=$2; shift 2 ;;
        --third-party) third=$2; shift 2 ;;
        *) printf 'package-release.sh: unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done
[ -n "$binary" ] && [ -n "$os" ] && [ -n "$arch" ] && [ -n "$out" ] || {
    printf '%s\n' 'usage: package-release.sh --binary PATH --os OS --arch ARCH --out DIR [--version V] [--third-party FILE]' >&2
    exit 2
}
case "$os" in macos|linux) ;; *) printf 'unsupported os: %s\n' "$os" >&2; exit 2 ;; esac
case "$arch" in aarch64|x86_64) ;; *) printf 'unsupported arch: %s\n' "$arch" >&2; exit 2 ;; esac
[ -f "$binary" ] || { printf 'no such binary: %s\n' "$binary" >&2; exit 2; }
[ -n "$third" ] || third=$repo_root/THIRD_PARTY_LICENSES.html
[ -f "$third" ] || {
    printf 'no third-party licenses file: %s (generate it with: cargo about generate --locked about.hbs -o THIRD_PARTY_LICENSES.html)\n' "$third" >&2
    exit 2
}
if [ -z "$version" ]; then
    version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$repo_root/Cargo.toml" | head -n 1)
fi
[ -n "$version" ] || { printf '%s\n' 'cannot determine the package version' >&2; exit 2; }

mkdir -p "$out"
out=$(CDPATH='' cd "$out" && pwd -P)
staging=$(mktemp -d "${TMPDIR:-/tmp}/ht-package.XXXXXX")
trap 'rm -rf "$staging"' EXIT HUP INT TERM
package="$staging/herdr-threads"
mkdir -p "$package/bin" "$package/scripts" "$package/integrations/claude" "$package/integrations/codex"
cp "$repo_root/herdr-plugin.toml" "$package/"
cp "$repo_root/README.md" "$repo_root/LICENSE-MIT" "$repo_root/LICENSE-APACHE" "$package/"
cp "$third" "$package/THIRD_PARTY_LICENSES.html"
cp "$repo_root/scripts/view.sh" "$repo_root/scripts/build.sh" "$package/scripts/"
cp "$repo_root/integrations/claude/README.md" "$package/integrations/claude/"
cp "$repo_root/integrations/codex/README.md" "$package/integrations/codex/"
cp "$binary" "$package/bin/herdr-threads"
chmod 755 "$package/bin/herdr-threads" "$package/scripts/view.sh" "$package/scripts/build.sh"
printf '%s\n' "$version" > "$package/VERSION"
printf '%s-%s\n' "$os" "$arch" > "$package/TARGET"
printf '%s\n' 'Prebuilt release package: scripts/build.sh keeps bin/herdr-threads.' > "$package/PREBUILT"

archive="$out/herdr-threads-$os-$arch.tar.gz"
# No extended attributes or AppleDouble files from macOS tar.
COPYFILE_DISABLE=1 tar -czf "$archive.tmp" -C "$staging" herdr-threads
mv -f "$archive.tmp" "$archive"
printf '%s\n' "$archive"

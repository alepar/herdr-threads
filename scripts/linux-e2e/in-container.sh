#!/usr/bin/env bash
# Steps scripts/linux-e2e/run executes inside its container (image: scripts/linux-e2e/Dockerfile).
# Not meant for a host: it assumes the image layout (/home/tester/src holds the exported tree,
# /home/tester/evidence is copied out, /home/tester/run is scratch, /home/tester/src/target may be a
# cache volume) and a disposable container.
#
#   in-container.sh STEP [ARGS...]
#
# Steps: prepare-src TREE | self-test | release-smoke [VERSION...] | herdr-up | build-release |
#        install-release | daemon-wait | doctor-outside | pane-run NAME TIMEOUT_S CMD... |
#        build-bin | wake-e2e-pane | harness-snapshot NAME
set -euo pipefail

# HT_LE2E_HOME exists for scripts/tests/test_linux_e2e_run.py (the launcher's preflight on a host).
H=${HT_LE2E_HOME:-/home/tester}
SRC=$H/src
EV=$H/evidence
RUN=$H/run
CREDS=$H/.ht-creds
mkdir -p "$EV" "$RUN"
if [ "${HT_HIDE_HARNESSES:-}" = 1 ]; then
    # A Linux user without Claude Code or Codex: drop their bin dir from PATH.
    PATH=$(printf '%s' "$PATH" | tr ':' '\n' | grep -vx /opt/harnesses/bin | paste -sd: -)
    export PATH
fi

say() { printf 'in-container: %s\n' "$*"; }
die() { printf 'in-container: %s\n' "$*" >&2; exit 1; }
arch() { read -r a _ < /etc/ht-arch; printf '%s' "$a"; }
version() { sed -n 's/^version = "\(.*\)"$/\1/p' "$SRC/Cargo.toml" | head -n 1; }

step_prepare_src() { # SHA TREE: make the exported tree a checkout of SHA itself (a one-commit shallow repo)
    local sha=$1 tree=$2
    cd "$SRC"
    # The index holds exactly the archived files; its tree id must equal the host's HEAD^{tree}, which proves
    # the export complete and exact. The host's raw commit object ($RUN/export-commit) over that tree then
    # hashes to SHA, so `git rev-parse HEAD` (recorded by the e2e scripts) names the commit under test and
    # `git status` is clean. Its parents are absent, recorded as a shallow boundary.
    git init -q
    git add -A -f -- . ':!target'
    local actual
    actual=$(git write-tree)
    [ "$actual" = "$tree" ] || die "exported tree $actual differs from the host's HEAD tree $tree"
    local commit
    commit=$(git hash-object -t commit -w --stdin < "$RUN/export-commit")
    [ "$commit" = "$sha" ] || die "rebuilt commit $commit differs from the host's HEAD $sha"
    printf '%s\n' "$sha" > .git/shallow
    git update-ref HEAD "$sha"
    [ "$(git rev-parse HEAD)" = "$sha" ] && [ -z "$(git status --porcelain)" ] || die "checkout of $sha is not clean"
    printf 'host HEAD %s\ntree %s\n' "$sha" "$actual" > "$EV/source.txt"
    say "source: HEAD $sha, tree $actual (git archive export, verified)"
}

step_self_test() {
    cd "$SRC"
    python3 scripts/wake-e2e.py --self-test
}

step_release_smoke() { # [VERSION...]
    cd "$SRC"
    local tmp
    # Short TMPDIR (tests bind Unix sockets; 108-byte limit), with a prefix the leak checker scopes.
    tmp=$(mktemp -d /tmp/htci.XXXXXX)
    export TMPDIR=$tmp
    HT_LEAK_RUN_ID=${HT_LEAK_RUN_ID:-linux-e2e-$(cat /proc/sys/kernel/random/uuid)}
    export HT_LEAK_RUN_ID
    local cache=$H/.cache/herdr-releases
    local args=(--cache "$cache" --report "$EV/release-smoke.md")
    [ $# -gt 0 ] && args+=(--versions "$@")
    local rc=0
    # The cache may be a volume shared by concurrent runs: one smoke at a time per cache, so no two
    # downloads race on the same .part file (cargo locks target/ itself).
    flock "$cache/.lock" scripts/herdr-release-smoke "${args[@]}" || rc=$?
    scripts/check-no-leaked-processes --run-id "$HT_LEAK_RUN_ID" --grace 15 > "$EV/leak-check.txt" 2>&1 \
        || { rc=1; cat "$EV/leak-check.txt"; }
    return "$rc"
}

socket_path() { sed -n 's/.*api socket: *//p' "$EV/herdr-server.log" | head -n 1; }

step_herdr_up() { # a headless Herdr server for this container (its own; the host's is never involved)
    if [ -f "$RUN/herdr-server.pid" ] && kill -0 "$(cat "$RUN/herdr-server.pid")" 2>/dev/null; then
        say "herdr server already up"; return 0
    fi
    # Panes inherit this server's environment: HT_LINUX_E2E_ENV_PROBE lets the pane launcher record
    # that a value given to the server reaches the agents (how a Claude token is meant to travel).
    # HT_LINUX_E2E_CONTAINER tells scripts/wake-e2e.py it runs in this disposable container (with
    # /.dockerenv), the only place its linux mode bypasses the Codex sandbox.
    HT_LINUX_E2E_ENV_PROBE=1 HT_LINUX_E2E_CONTAINER=1 setsid nohup herdr server </dev/null > "$EV/herdr-server.log" 2>&1 &
    echo $! > "$RUN/herdr-server.pid"
    local i=0 sock
    while [ $i -lt 150 ]; do
        sock=$(socket_path)
        if [ -n "$sock" ] && [ -S "$sock" ]; then
            printf '%s\n' "$sock" > "$RUN/herdr-socket"
            say "herdr $(herdr --version | awk '{print $2}') server up: $sock"
            return 0
        fi
        kill -0 "$(cat "$RUN/herdr-server.pid")" 2>/dev/null || die "herdr server exited (see herdr-server.log)"
        sleep 0.1; i=$((i + 1))
    done
    die "herdr server did not create its socket in 15 s"
}

step_build_release() { # the static musl release archive and a file:// release tree, as release.yml does
    cd "$SRC"
    local a target v
    a=$(arch); target="$a-unknown-linux-musl"; v=$(version)
    cc_var="CC_$(printf '%s' "$target" | tr '-' '_')"
    export "$cc_var=musl-gcc"
    cargo build --release --locked --target "$target"
    local binary="target/$target/release/herdr-threads"
    file -L "$binary" | tee "$EV/release-binary-file.txt"
    file -L "$binary" | grep -Eq 'statically linked|static-pie linked' || die "release binary is not static"
    test "$("$binary" --version)" = "herdr-threads $v" || die "release binary reports the wrong version"
    # release.yml generates THIRD_PARTY_LICENSES.html with cargo-about; the runner ships a marked
    # placeholder instead (the archive layout is what install.sh and the plugin read).
    local third="$RUN/THIRD_PARTY_LICENSES.html"
    printf '<!-- placeholder: scripts/linux-e2e does not run cargo-about -->\n' > "$third"
    rm -rf "$RUN/dist" "$RUN/rel"
    scripts/package-release.sh --binary "$binary" --os linux --arch "$a" --version "$v" \
        --out "$RUN/dist" --third-party "$third"
    local tree="$RUN/rel/download/v$v"
    mkdir -p "$tree"
    cp "$RUN/dist/herdr-threads-linux-$a.tar.gz" "$tree/"
    (cd "$tree" && sha256sum herdr-threads-*.tar.gz > SHA256SUMS && cat SHA256SUMS) | tee "$EV/release-sha256sums.txt"
}

step_install_release() { # scripts/install.sh from the file:// release tree, like a Linux user
    cd "$SRC"
    local v; v=$(version)
    HERDR_THREADS_RELEASE_URL="file://$RUN/rel" bash scripts/install.sh --version "v$v" --setup </dev/null
    herdr-threads --version
    herdr plugin list || true
}

step_daemon_wait() { # first reconcile: doctor reports a transient `degraded` before it (scoping check 2)
    local i=0 out
    while [ $i -lt 60 ]; do
        out=$(herdr-threads daemon health 2>&1 || true)
        if printf '%s\n' "$out" | grep -Eq '(^|[^[:alpha:]])healthy'; then  # never "unhealthy"
            printf '%s\n' "$out" > "$EV/daemon-health.txt"
            say "daemon healthy after ${i}s"; return 0
        fi
        sleep 1; i=$((i + 1))
    done
    say "daemon not healthy after 60 s: $out"
}

step_doctor_outside() {
    { echo "### environment (HERDR_* only)"; env -u HERDR_ENV -u HERDR_PANE_ID bash -c 'env | grep "^HERDR_" | sort' || true
      echo; echo "### doctor (outside a Herdr pane)"; env -u HERDR_ENV -u HERDR_PANE_ID herdr-threads doctor; echo "rc=$?"
      echo; echo "### doctor --debug (outside a Herdr pane)"; env -u HERDR_ENV -u HERDR_PANE_ID herdr-threads doctor --debug; echo "rc=$?"
    } 2>&1 | tee "$EV/doctor-outside.txt"
}

workspace_pane() { # the runner's one workspace on the container's Herdr server; prints its root pane
    if [ -s "$RUN/pane" ]; then cat "$RUN/pane"; return 0; fi
    herdr workspace create --label ht-linux-e2e --cwd "$SRC" --no-focus > "$RUN/workspace.json"
    python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["result"]["root_pane"]["pane_id"])' \
        "$RUN/workspace.json" | tee "$RUN/pane"
}

step_pane_run() { # NAME TIMEOUT_S CMD...: run CMD in the workspace pane, wait, mirror its exit status
    local name=$1 timeout=$2; shift 2
    local pane job="$RUN/$name.job.sh" rcfile="$RUN/$name.rc"
    pane=$(workspace_pane)
    rm -f "$rcfile"
    {
        echo '#!/usr/bin/env bash'
        printf 'cd %q\n' "$SRC"
        printf '{ echo "### environment (HERDR_* only)"; env | grep "^HERDR_" | sort; echo; %s; } 2>&1 | tee %q\n' \
            "$(printf '%q ' "$@")" "$EV/$name.txt"
        # shellcheck disable=SC2016 # expands when the job runs
        printf 'echo "${PIPESTATUS[0]}" > %q\n' "$rcfile.tmp"
        printf 'mv %q %q\n' "$rcfile.tmp" "$rcfile"
    } > "$job"
    chmod 700 "$job"
    herdr pane run "$pane" "bash $job" > /dev/null
    local waited=0
    while [ ! -f "$rcfile" ]; do
        [ "$waited" -lt "$timeout" ] || { herdr pane read "$pane" > "$EV/$name.screen.txt" 2>&1 || true
                                          die "$name: no exit status after ${timeout}s in pane $pane"; }
        sleep 1; waited=$((waited + 1))
    done
    herdr pane read "$pane" > "$EV/$name.screen.txt" 2>&1 || true
    local rc; rc=$(cat "$rcfile")
    say "$name finished in pane $pane: exit $rc"
    return "$rc"
}

step_build_bin() { # bin/herdr-threads, what wake-e2e drives (--bin default)
    cd "$SRC"
    scripts/build.sh
    bin/herdr-threads --version
}

step_harness_snapshot() { # NAME: which harness processes run right now (none expected without credentials)
    { echo "### harness processes at $(date -u +%FT%TZ)"
      pgrep -a -f '(^|/)(claude|codex)( |$)' || echo "none"
    } | tee "$EV/$1.txt"
}

driver_harnesses() { # the --harness value among the driver args (default claude,codex)
    local next=0 value=claude,codex
    for word in "$@"; do
        if [ "$next" = 1 ]; then value=$word; next=0; continue; fi
        case "$word" in --harness) next=1 ;; --harness=*) value=${word#--harness=} ;; esac
    done
    printf '%s' "$value" | tr ',' ' '
}

step_wake_e2e_pane() { # the launcher typed into the pane: credential preflight, then the driver
    local args=()
    local a  # bash 3.2-compatible reads and empty-array expansions: the self-test runs this on macOS too
    if [ -s "$RUN/driver-args" ]; then while IFS= read -r -d '' a; do args+=("$a"); done < "$RUN/driver-args"; fi
    local problems=() h
    [ "${HERDR_ENV:-}" = 1 ] && [ -n "${HERDR_PANE_ID:-}" ] || problems+=("not inside a Herdr pane (HERDR_ENV/HERDR_PANE_ID unset)")
    echo "server environment reaches panes: $([ "${HT_LINUX_E2E_ENV_PROBE:-}" = 1 ] && echo yes || echo NO)"
    for h in $(driver_harnesses ${args[@]+"${args[@]}"}); do
        case "$h" in
            claude)
                if [ -n "${CLAUDE_CODE_OAUTH_TOKEN:-}" ]; then echo "claude credential: CLAUDE_CODE_OAUTH_TOKEN (set, not shown)"
                elif [ -n "${ANTHROPIC_API_KEY:-}" ]; then echo "claude credential: ANTHROPIC_API_KEY (set, not shown)"
                else problems+=("claude: no credential (run --claude-oauth-token-file FILE or --anthropic-api-key-file FILE)"); fi ;;
            codex)
                if [ -s "$CREDS/codex/auth.json" ]; then echo "codex credential: staged auth.json (mode $(stat -c %a "$CREDS/codex/auth.json" 2>/dev/null || stat -f %Lp "$CREDS/codex/auth.json"))"
                else problems+=("codex: no credential (run --codex-auth-file FILE)"); fi ;;
            *) problems+=("unknown harness: $h") ;;
        esac
    done
    [ -f "$RUN/live" ] || problems+=("live run not requested (run --live: it spends Claude/Codex quota)")
    if [ ${#problems[@]} -gt 0 ]; then
        echo "wake-e2e: refused before starting any harness:"
        printf '  - %s\n' "${problems[@]}"
        step_harness_snapshot harnesses-at-refusal > /dev/null
        echo "harness processes: $(tail -n +2 "$EV/harnesses-at-refusal.txt" | tr '\n' ' ')"
        return 2
    fi
    local extra=(--bin "$SRC/bin/herdr-threads" --evidence-dir "$EV/wake-e2e")  # the runner owns both
    if [ -s "$CREDS/codex/auth.json" ]; then extra+=(--codex-auth-home "$CREDS/codex"); fi
    cd "$SRC"
    exec python3 scripts/wake-e2e.py "${extra[@]}" ${args[@]+"${args[@]}"}
}

[ $# -ge 1 ] || die "usage: in-container.sh STEP [ARGS...]"
name=$1; shift
case "$name" in
    prepare-src|self-test|release-smoke|herdr-up|build-release|install-release|daemon-wait|doctor-outside|\
    pane-run|build-bin|wake-e2e-pane|harness-snapshot)
        "step_${name//-/_}" "$@" ;;
    *) die "unknown step: $name" ;;
esac

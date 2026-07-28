#!/bin/sh
# End-to-end checks against a built `aura` binary.
#
# Everything here is a claim the documentation makes, checked against the real
# executable rather than the library: exit codes, capability refusals, hermetic
# mode, output formats and determinism. The unit and conformance suites run
# in-process and so cannot catch a binary that was cross-compiled and never run,
# or one that fails to start on an older libc.
#
# Deliberately POSIX sh with no dependencies beyond coreutils: it has to run on
# Alpine's ash and inside a scratch-like container, which is the point — a clean
# system is where a broken artifact shows up.
#
# Usage:  ./e2e.sh /path/to/aura      (or with `aura` already on PATH)

set -eu

AURA="${1:-aura}"
command -v "$AURA" >/dev/null 2>&1 || [ -x "$AURA" ] || {
    echo "e2e: no such binary: $AURA" >&2
    exit 2
}

# Resolve to an absolute path before anything else: the fixtures live in a temp
# directory this script cd's into, and a relative path like ./aura would stop
# resolving there — every check would then fail with 127 and blame the binary.
case "$AURA" in
*/*) AURA="$(cd "$(dirname "$AURA")" && pwd)/$(basename "$AURA")" ;;
*) AURA="$(command -v "$AURA")" ;;
esac

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"

PASS=0
FAIL=0

# `ok <description>` / `no <description> <detail>` keep the output greppable in a
# CI log, where this may be the only thing a maintainer sees.
ok() {
    PASS=$((PASS + 1))
    echo "ok   - $1"
}
# Both helpers must end with a zero status. Under `set -e` a helper returning 1
# aborts the whole run at the first failure — which is exactly the case (a badly
# broken binary) where every remaining check is worth seeing.
no() {
    FAIL=$((FAIL + 1))
    echo "FAIL - $1"
    if [ $# -gt 1 ]; then
        echo "       $2"
    fi
    return 0
}

# Runs the binary, capturing stdout, stderr and the exit code for inspection.
run() {
    set +e
    "$AURA" "$@" >"$WORK/.out" 2>"$WORK/.err"
    RC=$?
    set -e
    OUT=$(cat "$WORK/.out")
    ERR=$(cat "$WORK/.err")
}

expect_rc() {
    if [ "$RC" = "$1" ]; then
        ok "$2"
    else
        no "$2" "expected exit $1, got $RC; stderr: $(echo "$ERR" | head -1)"
    fi
}

# The diagnostic code must appear in the output. stderr is rendered with colour,
# so the code can be split by escape sequences — strip anything non-printable
# before looking, or this silently never matches.
expect_code() {
    plain=$(printf '%s%s' "$OUT" "$ERR" | tr -d '\033' | sed 's/\[[0-9;]*m//g')
    case "$plain" in
    *"$1"*) ok "$2" ;;
    *) no "$2" "expected $1; got: $(echo "$plain" | tr -d '\n' | cut -c1-140)" ;;
    esac
}

expect_out() {
    case "$OUT" in
    *"$1"*) ok "$2" ;;
    *) no "$2" "expected to find $1 in: $(echo "$OUT" | tr -d '\n' | cut -c1-140)" ;;
    esac
}

# ---------------------------------------------------------------- fixtures

cat >app.aura <<'EOF'
api:
  port: 8080
  name: "checkout"
end
EOF

cat >broken.aura <<'EOF'
api:
  port: undefined_name
end
EOF

cat >reads.aura <<'EOF'
data: read_file("secret.txt")
EOF

cat >main.aura <<'EOF'
import "dep.aura" as dep

value: dep.data
EOF

cat >dep.aura <<'EOF'
data: read_file("secret.txt")
EOF

cat >uses_env.aura <<'EOF'
home: env("HOME", "/")
EOF

cat >unformatted.aura <<'EOF'
api:
      port:    8080
end
EOF

echo "s3cr3t" >secret.txt

# ---------------------------------------------------------------- the checks

echo "# aura e2e: $($AURA --version 2>&1 | head -1)"

run --version
expect_rc 0 "--version succeeds"
expect_out "aura" "--version names the binary"

run eval app.aura
expect_rc 0 "eval of a valid manifest succeeds"
expect_out '"port": 8080' "eval emits the evaluated value"

# D13. The headline determinism claim, checked on the artifact: nothing else
# does, and a nondeterministic map iteration would pass every in-process test
# that only ever runs once.
run eval app.aura
first=$OUT
run eval app.aura
if [ "$first" = "$OUT" ]; then
    ok "two runs are byte-identical (D13)"
else
    no "two runs are byte-identical (D13)" "output differed between runs"
fi

run eval --format yaml app.aura
expect_rc 0 "--format yaml succeeds"
expect_out "port: 8080" "--format yaml emits YAML"

run eval --format toml app.aura
expect_rc 0 "--format toml succeeds"

run eval --format json-flat app.aura
expect_out '"api.port"' "--format json-flat flattens nested keys"

run check app.aura
expect_rc 0 "check accepts a valid manifest"

run check broken.aura
expect_rc 1 "check rejects an undefined name"
expect_code "E0504" "the undefined name is reported as E0504"

run eval missing-file.aura
expect_rc 2 "a missing file is exit 2, not 1"

# The capability model, which is the language's central claim.
run eval reads.aura
expect_rc 1 "read_file without a grant fails"
expect_code "E0310" "the refusal is E0310"

run eval --allow-read=. reads.aura
expect_rc 0 "read_file succeeds once granted"

# D1: the grant belongs to the root manifest and does not reach an import — the
# one property most worth verifying on the shipped binary.
run eval --allow-read=. main.aura
expect_rc 1 "an import cannot read files even when the root was granted"
expect_code "E0310" "the import's refusal is E0310"

# Hermetic mode is decided statically, so `check` alone must reject it without
# evaluating anything.
run check --hermetic uses_env.aura
expect_rc 1 "check --hermetic rejects env()"
expect_code "E0505" "the hermetic refusal is E0505"

run check --hermetic app.aura
expect_rc 0 "check --hermetic accepts a manifest that performs no I/O"

run fmt --check app.aura
expect_rc 0 "fmt --check accepts canonical formatting"

run fmt --check unformatted.aura
expect_rc 1 "fmt --check rejects non-canonical formatting"

# ---------------------------------------------------------------- summary

echo
echo "# passed: $PASS, failed: $FAIL"
[ "$FAIL" -eq 0 ] || exit 1

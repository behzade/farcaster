#!/bin/sh
# Opt-in live tests: real installed harnesses and real model accounts.
# Every case gets a separate process and Farcaster database. Native harness
# credentials remain available; this script never changes HOME or CODEX_HOME.
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-"$repo_root/target"}

case ${HARNESS:-} in
    "") harnesses="opencode2 codex-cli pi claude cursor-cli antigravity-acp" ;;
    opencode2|codex-cli|pi|claude|cursor-cli|antigravity-acp) harnesses=$HARNESS ;;
    *) echo "Unknown HARNESS: $HARNESS" >&2; exit 2 ;;
esac

artifact_parent=${FARCASTER_E2E_ARTIFACT_PARENT:-${TMPDIR:-/tmp}}
run_dir=$(mktemp -d "$artifact_parent/farcaster-live-e2e.XXXXXXXX")
printf 'Live model tests; evidence: %s\n' "$run_dir"
printf 'harness\tcase\tresult\tseconds\tlog\n' > "$run_dir/results.tsv"
git rev-parse HEAD > "$run_dir/commit.txt"
git status --short > "$run_dir/worktree.txt"
git diff --binary HEAD > "$run_dir/worktree.patch"

# Build once, then clone that exact binary. Concurrent workspace edits must not
# change the implementation halfway through a harness's feature matrix.
if ! python3 scripts/run-live-case.py --timeout "${FARCASTER_E2E_BUILD_TIMEOUT:-900}" -- \
    cargo test --locked --bin farcaster --no-run --message-format=json \
    < /dev/null > "$run_dir/build.jsonl" 2> "$run_dir/build.log"; then
    echo "E2E build/discovery failed; no live result claimed." >&2
    tail -n 70 "$run_dir/build.log" >&2
    python3 -c '
import json, sys
for line in sys.stdin:
    try:
        event = json.loads(line)
    except json.JSONDecodeError:
        continue
    message = event.get("message", {})
    if event.get("reason") == "compiler-message" and message.get("level") == "error":
        print(message.get("rendered", message.get("message", "compiler error")), file=sys.stderr)
' < "$run_dir/build.jsonl"
    exit 1
fi
git diff --binary HEAD > "$run_dir/worktree-after-build.patch"
if ! cmp -s "$run_dir/worktree.patch" "$run_dir/worktree-after-build.patch"; then
    echo "Source changed during the E2E build; freeze the worktree and retry. No live result claimed." >&2
    exit 1
fi
built_binary=$(python3 -c '
import json, sys
paths = set()
for line in sys.stdin:
    event = json.loads(line)
    if (event.get("reason") == "compiler-artifact"
        and event.get("target", {}).get("name") == "farcaster"
        and event.get("profile", {}).get("test")
        and event.get("executable")):
        paths.add(event["executable"])
if len(paths) != 1:
    sys.exit("Expected exactly one compiled Farcaster test executable")
print(paths.pop())
' < "$run_dir/build.jsonl")
frozen_binary="$run_dir/farcaster-tests"
# APFS clones retain the inode contents without copying 200+ MB per harness.
cp -c "$built_binary" "$frozen_binary" 2>/dev/null || cp "$built_binary" "$frozen_binary"
set -- "$frozen_binary"
if [ "$(uname -s)" = Darwin ]; then
    # The prescribed runner never executes from the crowded deps directory.
    set -- "$repo_root/scripts/run-macos.sh" "$frozen_binary"
fi
if ! python3 scripts/run-live-case.py --timeout 60 -- \
    "$@" live_e2e_ --ignored --list > "$run_dir/test-list.txt" 2>> "$run_dir/build.log"; then
    echo "Frozen binary discovery failed; no live result claimed." >&2
    tail -n 40 "$run_dir/build.log" >&2
    exit 1
fi
sed -n '/::live_e2e_[^:]*: test$/s/: test$//p' "$run_dir/test-list.txt" |
    awk '
        { priority = 60 }
        /::live_basic_tests::/ { priority = 10 }
        /::live_e2e_regular_message_response/ { priority = 0 }
        /::live_input_tests::/ { priority = 20 }
        /::runtime::live_e2e_tests::/ { priority = 30 }
        /::live_children_tests::/ { priority = 40 }
        /::app::live_e2e_tests::/ { priority = 50 }
        { print priority "\t" $0 }
    ' | LC_ALL=C sort -k1,1n -k2,2 | cut -f2- > "$run_dir/cases.txt"
if [ ! -s "$run_dir/cases.txt" ]; then
    echo "No live E2E feature tests found; refusing an empty green run." >&2
    exit 1
fi

failed=0
limited=0
ran=0
for harness in $harnesses; do
    mkdir "$run_dir/$harness"
    while IFS= read -r test_name; do
        case "$test_name" in
            *"${CASE:-}"*) ;;
            *) continue ;;
        esac
        case_name=${test_name##*::}
        case_dir="$run_dir/$harness/$case_name"
        mkdir "$case_dir" "$case_dir/data" "$case_dir/evidence"
        started=$(date +%s)
        printf '%s %s: RUNNING\n' "$harness" "$case_name"
        # Isolation is mandatory and owned by this invocation, even if the caller
        # has FARCASTER_DATA_DIR pointing at their normal application database.
        if FARCASTER_DATA_DIR="$case_dir/data" \
            FARCASTER_E2E_ARTIFACT_DIR="$case_dir/evidence" \
            FARCASTER_E2E_HARNESS="$harness" \
            python3 scripts/run-live-case.py --timeout "${FARCASTER_E2E_CASE_TIMEOUT:-600}" -- \
            "$@" "$test_name" --exact --ignored --nocapture --test-threads=1 \
                < /dev/null > "$case_dir/output.log" 2>&1; then
            if grep -q 'test result: ok. 1 passed; 0 failed; 0 ignored;' "$case_dir/output.log"; then
                result=PASS
                if grep -q 'E2E_LIMIT:' "$case_dir/output.log"; then
                    result=LIMITED
                    limited=$((limited + 1))
                fi
            else
                result=FAIL
                failed=$((failed + 1))
                echo 'A successful command without one executed test is not coverage.' >> "$case_dir/output.log"
            fi
        else
            result=FAIL
            if grep -q 'E2E_BLOCKED:' "$case_dir/output.log"; then result=BLOCKED; fi
            if grep -q 'E2E_TIMEOUT:' "$case_dir/output.log"; then result=TIMEOUT; fi
            failed=$((failed + 1))
        fi
        elapsed=$(($(date +%s) - started))
        ran=$((ran + 1))
        printf '%s\t%s\t%s\t%s\t%s\n' "$harness" "$case_name" "$result" \
            "$elapsed" "$case_dir/output.log" >> "$run_dir/results.tsv"
        printf '%s %s: %s (%ss)\n' "$harness" "$case_name" "$result" "$elapsed"
        if [ "$result" = LIMITED ]; then
            grep 'E2E_LIMIT:' "$case_dir/output.log"
        elif [ "$result" != PASS ]; then
            # The full trace stays in output.log; do not print a many-thousand
            # character JSON event line as the per-feature summary.
            tail -n 14 "$case_dir/output.log" | cut -c1-1000
        fi
    done < "$run_dir/cases.txt"
done

if [ "$ran" -eq 0 ]; then
    echo "CASE matched no tests; refusing an empty green run." >&2
    exit 1
fi
printf 'Completed %s cases; %s failed/blocked; %s limited. Evidence: %s\n' \
    "$ran" "$failed" "$limited" "$run_dir"
# A partial capability proof is useful, but not a fully green feature matrix.
[ "$failed" -eq 0 ] && [ "$limited" -eq 0 ]

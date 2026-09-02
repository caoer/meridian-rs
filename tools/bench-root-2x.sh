#!/bin/bash
# Build and stamp a 2x wall-clock bench root.
#
# THE TRAP THIS EXISTS TO CLOSE: building a scaled root by nesting a copy of
# the 1x tree (dup/, mirror-b/, root2x/a+b) escapes the ROOT-RELATIVE ignore
# rules in the root's domain config. Members the 1x domain excludes — including
# non-UTF-8 fixtures — re-enter at their nested paths, where no rule matches
# them.
#
# THE TRAP IS QUIET. The engine refuses invalid_utf8 PER FILE at read time —
# the corpus serves, zero-reads answer no_effect, and only a program touching
# the escaped member refuses. Nothing announces the escape at serve time, and a
# mis-scaled root distorts figures silently either way. A mechanical stamp is
# therefore the ONLY detector; eyeballing hello is not evidence of a valid root.
#
# THE SAFE RECIPE: never copy trees. `construct` asks the engine for the 1x
# domain member list (the same hash_domain the daemon folds) and copies exactly
# those files, twice, under a/ and b/. Excluded members are never copied, so no
# ignore rule needs re-spelling at any nested prefix, and the 2x domain is 2x
# the 1x member list by construction.
#
# THE STAMP: `stamp` is the mandatory rig receipt, emitted BEFORE any
# measurement. Four legs, all required:
#   1. member ratio — 2x/1x within 2.00 +/- 0.01, by the engine's own
#      enumeration (the floor; catches gross mis-scale);
#   2. exact member set — the 2x list must be PRECISELY a/<m> + b/<m> over the
#      1x list. The load-bearing leg: the live escape added 2 members (ratio
#      2.0001, inside any usable tolerance — field-valid roots measured
#      1.996), and set equality catches every escape member for member;
#   3. served/warm receipt — a zero-read program (`1 + 1`) must answer
#      no_effect (catches an unservable corpus, and leaves the daemon warm for
#      the measurement that follows);
#   4. engine identity — the socket holder resolved via lsof must be the MRD
#      binary under test, so a receipt can never silently come from the
#      user's installed engine (an ambient XDG_CACHE_HOME routes the receipt
#      to the resident daemon).
# The stamp FILE is written either way — on failure it is the refusal receipt,
# and the exit code refuses the measurement. It lives BESIDE the root, never
# inside it: an extra .md inside would join the domain and skew the count.
#
# usage:
#   bench-root-2x.sh construct <1x-root> <out-2x-root>
#   bench-root-2x.sh stamp <1x-root> <2x-root> [stamp-file]
#
# env:
#   DOMAIN_MEMBERS  prebuilt member-list binary
#                   (target/release/examples/domain_members; building it needs
#                   the build slot — this script never builds)
#   MRD             prebuilt mrd binary (stamp only)
#   BENCH_XDG       cache/socket isolation root for the stamp's daemon;
#                   defaults to <2x-root>.xdg — a sibling, outside the root.
#                   The ambient XDG_CACHE_HOME is deliberately IGNORED: it is
#                   set in nearly every user shell, and inheriting it sends the
#                   receipt through the user's resident registry socket, where
#                   the installed engine answers instead of the binary under
#                   test
set -euo pipefail

die() {
    echo "bench-root-2x: $*" >&2
    exit 1
}

need_members_bin() {
    [ -n "${DOMAIN_MEMBERS:-}" ] && [ -x "$DOMAIN_MEMBERS" ] ||
        die "DOMAIN_MEMBERS must name the prebuilt domain_members binary (cargo build --release -p fs --example domain_members — needs the build slot)"
}

# Copy the root-level domain config, whichever surface the 1x root uses. The
# rules are root-relative, so the config must exist AT the new root for the 2x
# domain to be loadable at all; with member-list construction its ignore rules
# match nothing (nothing excluded was copied), which is the point.
copy_config() {
    local one=$1 out=$2
    if [ -f "$one/meridian/domain.md" ]; then
        [ -f "$one/mdfs_config.yaml" ] && die "two domain configs present in $one — the engine refuses this root"
        mkdir -p "$out/meridian"
        cp "$one/meridian/domain.md" "$out/meridian/"
    elif [ -f "$one/mdfs_config.yaml" ]; then
        cp "$one/mdfs_config.yaml" "$out/"
    fi
}

construct() {
    local one=$1 out=$2
    need_members_bin
    [ -d "$one" ] || die "1x root not found: $one"
    [ -e "$out" ] && die "refusing to overwrite $out"
    local list
    list=$(mktemp)
    "$DOMAIN_MEMBERS" "$one" >"$list"
    [ -s "$list" ] || die "$one yields no domain members"
    mkdir -p "$out/a" "$out/b"
    rsync -a --files-from="$list" "$one/" "$out/a/"
    rsync -a --files-from="$list" "$one/" "$out/b/"
    copy_config "$one" "$out"
    echo "constructed $out: 2 x $(wc -l <"$list" | tr -d ' ') members from $one"
    rm -f "$list"
    echo "next: stamp it — $0 stamp $one $out"
}

stamp() {
    local one=$1 two=$2
    local file=${3:-$two.stamp.md}
    need_members_bin
    [ -n "${MRD:-}" ] && [ -x "$MRD" ] || die "MRD must name a prebuilt mrd binary"
    [ -d "$one" ] || die "1x root not found: $one"
    [ -d "$two" ] || die "2x root not found: $two"
    case "$file" in
    "$two"/*) die "stamp file must not live inside the 2x root (an .md member would skew the count)" ;;
    esac

    local one_list two_list expect_list
    one_list=$(mktemp) two_list=$(mktemp) expect_list=$(mktemp)
    "$DOMAIN_MEMBERS" "$one" | sort >"$one_list"
    "$DOMAIN_MEMBERS" "$two" | sort >"$two_list"
    local n1 n2 ratio ratio_verdict
    n1=$(wc -l <"$one_list" | tr -d ' ')
    n2=$(wc -l <"$two_list" | tr -d ' ')
    [ "$n1" -gt 0 ] || die "$one yields no domain members"
    ratio=$(awk -v a="$n1" -v b="$n2" 'BEGIN{printf "%.4f", b/a}')
    ratio_verdict=$(awk -v r="$ratio" 'BEGIN{print (r>=1.99 && r<=2.01) ? "ok" : "FAIL"}')

    # The exact leg, and it is the load-bearing one: the 2x member set must be
    # PRECISELY a/<m> + b/<m> over the 1x member list (plus the root-level
    # domain.md copy where that surface is in use). The ratio cannot catch a
    # small escape — field-valid roots measured 1.996 while the live trap
    # added only 2 members (2.0001) — and since the engine moved invalid_utf8
    # from a whole-corpus hello refusal to a per-file read-time refusal, an
    # escaped member no longer announces itself at serve time either. Set
    # equality catches every escape shape, member for member.
    {
        sed 's|^|a/|' "$one_list"
        sed 's|^|b/|' "$one_list"
        if [ -f "$two/meridian/domain.md" ]; then echo "meridian/domain.md"; fi
    } | sort >"$expect_list"
    local exact_verdict=FAIL escape_sample=""
    if diff -q "$expect_list" "$two_list" >/dev/null; then
        exact_verdict=ok
    else
        escape_sample=$(diff "$expect_list" "$two_list" | grep -E '^[<>]' | head -10 || true)
    fi
    rm -f "$one_list" "$two_list" "$expect_list"

    # This run's own daemon on its own socket: never the user's cache root,
    # and never the AMBIENT XDG_CACHE_HOME — it is set in nearly every user
    # shell and resolves the resident registry socket. A cold 47k-file
    # corpus build can outlive the client's spawn-ready timeout, so retry
    # rather than record a spawn failure as a refusal.
    export XDG_CACHE_HOME="${BENCH_XDG:-$two.xdg}"
    local served_verdict=FAIL receipt="" attempt
    for attempt in 1 2 3 4 5 6 7 8 9 10; do
        receipt=$(cd "$two" && printf '1 + 1' | "$MRD" script 2>&1) || true
        if printf '%s' "$receipt" | grep -q no_effect; then
            served_verdict=ok
            break
        fi
        sleep 3
    done

    # The engine leg: prove WHICH binary answered, from a capability source
    # (who holds the socket), never from the client's own version string — the
    # client prints its version whether or not it did the serving. No socket
    # after a served receipt means the ephemeral in-process path: the client
    # binary itself served, which is the binary under test by construction.
    local sock="$XDG_CACHE_HOME/meridian/registry/daemon.sock"
    local engine_verdict=FAIL daemon_pid=none daemon_binary=none daemon_inode=none
    if [ -S "$sock" ]; then
        daemon_pid=$(lsof -F p -- "$sock" 2>/dev/null | sed -n 's/^p//p' | head -1)
        if [ -n "$daemon_pid" ]; then
            daemon_binary=$(lsof -p "$daemon_pid" -a -d txt -F n 2>/dev/null |
                sed -n 's/^n//p' | grep -v '^/usr/lib/' | head -1)
            daemon_inode=$(lsof -p "$daemon_pid" -a -d txt -F i 2>/dev/null |
                sed -n 's/^i//p' | head -1)
            [ "$daemon_binary" = "$MRD" ] && engine_verdict=ok
        fi
    elif [ "$served_verdict" = ok ]; then
        daemon_pid=ephemeral
        daemon_binary=$MRD
        engine_verdict=ok
    fi

    local verdict=FAIL
    [ "$ratio_verdict" = ok ] && [ "$exact_verdict" = ok ] && [ "$served_verdict" = ok ] &&
        [ "$engine_verdict" = ok ] && verdict=PASS

    {
        echo "---"
        echo "type: rig-stamp"
        echo "two_x_root: \"$two\""
        echo "one_x_root: \"$one\""
        echo "members_1x: $n1"
        echo "members_2x: $n2"
        echo "ratio: $ratio"
        echo "ratio_verdict: $ratio_verdict"
        echo "exact_verdict: $exact_verdict"
        echo "served_verdict: $served_verdict"
        echo "engine_verdict: $engine_verdict"
        echo "verdict: $verdict"
        echo "mrd: \"$MRD\""
        echo "mrd_sha256: $(shasum -a 256 "$MRD" | cut -d' ' -f1)"
        echo "mrd_version: \"$("$MRD" --version)\""
        echo "daemon_pid: $daemon_pid"
        echo "daemon_binary: \"$daemon_binary\""
        echo "daemon_inode: $daemon_inode"
        echo "xdg_cache_home: \"$XDG_CACHE_HOME\""
        echo "stamped_at: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
        echo "attempts: $attempt"
        echo "---"
        echo
        echo "# 2x root-validity stamp"
        echo
        if [ -n "$escape_sample" ]; then
            echo "Member-set mismatch (\`<\` expected, \`>\` found; first 10):"
            echo
            echo '```'
            printf '%s\n' "$escape_sample"
            echo '```'
            echo
        fi
        echo "Last receipt line from the zero-read program:"
        echo
        echo '```'
        printf '%s\n' "$receipt" | tail -5
        echo '```'
    } >"$file"

    echo "stamp: members $n1 -> $n2, ratio $ratio ($ratio_verdict), exact $exact_verdict, served $served_verdict, engine $engine_verdict (pid $daemon_pid) -> $verdict ($file)"
    [ "$verdict" = PASS ] || die "stamp FAILED — this root must not be measured (receipt: $file)"
}

case "${1:-}" in
construct) construct "$2" "$3" ;;
stamp) stamp "$2" "$3" "${4:-}" ;;
*)
    sed -n '2,58p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
    ;;
esac

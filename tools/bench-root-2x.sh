#!/bin/bash
# Build and stamp a 2x wall-clock bench root.
#
# THE TRAP THIS EXISTS TO CLOSE (2026-08-08, all three wave-1 implementers hit
# it independently): building a scaled root by nesting a copy of the 1x tree
# (dup/, mirror-b/, root2x/a+b) escapes the ROOT-RELATIVE ignore rules in the
# root's domain config. Members the 1x domain excludes — including non-UTF-8
# fixtures — re-enter at their nested paths, where no rule matches them.
#   - Loud failure: one non-UTF-8 member makes the whole corpus unservable.
#     `hello` refuses invalid_utf8, the daemon never warms, and every reading
#     is a refusal with NO entry time — indistinguishable at a glance from the
#     wall-clock refusal under test.
#   - Quiet failure: the root serves but is wrongly scaled, so a gate figure
#     is measured at a scale the card does not name.
#
# THE SAFE RECIPE: never copy trees. `construct` asks the engine for the 1x
# domain member list (the same hash_domain the daemon folds) and copies exactly
# those files, twice, under a/ and b/. Excluded members are never copied, so no
# ignore rule needs re-spelling at any nested prefix, and the 2x domain is 2x
# the 1x member list by construction.
#
# THE STAMP: `stamp` is the mandatory rig receipt, emitted BEFORE any
# measurement. Two legs, both required, catching the two failure shapes:
#   1. member ratio — 2x members / 1x members within 2.00 +/- 0.01, counted by
#      the engine's own enumeration (catches the quiet mis-scale);
#   2. served/warm receipt — a zero-read program (`1 + 1`) against the 2x root
#      must answer no_effect (catches the unservable corpus, and leaves the
#      daemon warm for the measurement that follows).
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
#   XDG_CACHE_HOME  cache/socket isolation for the stamp's daemon; defaults to
#                   <2x-root>.xdg — a sibling, outside the root, and never the
#                   fleet's own cache root
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
    local one=$1 two=$2 file=${3:-$two.stamp.md}
    need_members_bin
    [ -n "${MRD:-}" ] && [ -x "$MRD" ] || die "MRD must name the seat's prebuilt mrd binary"
    [ -d "$one" ] || die "1x root not found: $one"
    [ -d "$two" ] || die "2x root not found: $two"
    case "$file" in
    "$two"/*) die "stamp file must not live inside the 2x root (an .md member would skew the count)" ;;
    esac

    local n1 n2 ratio ratio_verdict
    n1=$("$DOMAIN_MEMBERS" "$one" | wc -l | tr -d ' ')
    n2=$("$DOMAIN_MEMBERS" "$two" | wc -l | tr -d ' ')
    [ "$n1" -gt 0 ] || die "$one yields no domain members"
    ratio=$(awk -v a="$n1" -v b="$n2" 'BEGIN{printf "%.4f", b/a}')
    ratio_verdict=$(awk -v r="$ratio" 'BEGIN{print (r>=1.99 && r<=2.01) ? "ok" : "FAIL"}')

    # The seat's own daemon on the seat's own socket: never the fleet's cache
    # root. A cold 47k-file corpus build can outlive the client's spawn-ready
    # timeout, so retry rather than record a spawn failure as a refusal.
    export XDG_CACHE_HOME="${XDG_CACHE_HOME:-$two.xdg}"
    local served_verdict=FAIL receipt="" attempt
    for attempt in 1 2 3 4 5 6 7 8 9 10; do
        receipt=$(cd "$two" && printf '1 + 1' | "$MRD" script 2>&1) || true
        if printf '%s' "$receipt" | grep -q no_effect; then
            served_verdict=ok
            break
        fi
        sleep 3
    done

    local verdict=FAIL
    [ "$ratio_verdict" = ok ] && [ "$served_verdict" = ok ] && verdict=PASS

    {
        echo "---"
        echo "type: rig-stamp"
        echo "two_x_root: \"$two\""
        echo "one_x_root: \"$one\""
        echo "members_1x: $n1"
        echo "members_2x: $n2"
        echo "ratio: $ratio"
        echo "ratio_verdict: $ratio_verdict"
        echo "served_verdict: $served_verdict"
        echo "verdict: $verdict"
        echo "mrd: \"$MRD\""
        echo "mrd_sha256: $(shasum -a 256 "$MRD" | cut -d' ' -f1)"
        echo "mrd_version: \"$("$MRD" --version)\""
        echo "xdg_cache_home: \"$XDG_CACHE_HOME\""
        echo "stamped_at: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
        echo "attempts: $attempt"
        echo "---"
        echo
        echo "# 2x root-validity stamp"
        echo
        echo "Last receipt line from the zero-read program:"
        echo
        echo '```'
        printf '%s\n' "$receipt" | tail -5
        echo '```'
    } >"$file"

    echo "stamp: members $n1 -> $n2, ratio $ratio ($ratio_verdict), served $served_verdict -> $verdict ($file)"
    [ "$verdict" = PASS ] || die "stamp FAILED — this root must not be measured (receipt: $file)"
}

case "${1:-}" in
construct) construct "$2" "$3" ;;
stamp) stamp "$2" "$3" "${4:-}" ;;
*)
    sed -n '2,45p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
    ;;
esac

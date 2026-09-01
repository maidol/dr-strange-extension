#!/usr/bin/env bash
# Validate `catalog.json` — the official plugin list drsg reads at
# `drsg plugin install`.
#
# This file replaced a hard-coded table in the database's source tree, which
# means the compiler no longer checks it. This script is what took over that
# job, and it is deliberately strict: a wrong hash here does not fail a build,
# it fails an operator's install with "does not match the hash the official
# catalog pins", and a wrong URL fails it with a 404. Both are our bug, found
# by them.
#
#   ./scripts/check-catalog.sh            # shape only, no network
#   ./scripts/check-catalog.sh --online   # also check every artifact's hash
#
# `--online` fetches each release's published `<plugin>.wasm.sha256` and
# compares. It costs nine ~100-byte requests and is the only check that can
# catch the failure that matters most.
set -euo pipefail

cd "$(dirname "$0")/.."

online=0
[ "${1:-}" = "--online" ] && online=1

catalog="catalog.json"
fail=0
bad() {
    echo "catalog: $*" >&2
    fail=1
}

jq -e . "$catalog" >/dev/null || {
    echo "catalog: $catalog is not valid JSON" >&2
    exit 1
}

# The contract every entry is measured against: the canonical WIT's own
# package version, so the two cannot drift.
wit_contract=$(sed -n 's/^package drsg:preprocess@\(.*\);$/\1/p' wit/preprocess.wit)
[ -n "$wit_contract" ] || {
    echo "catalog: wit/preprocess.wit declares no versioned package" >&2
    exit 1
}

schema=$(jq -r '.schema // 0' "$catalog")
[ "$schema" = "1" ] || bad "schema is '$schema', expected 1"

count=$(jq '.plugins | length' "$catalog")
[ "$count" -gt 0 ] || bad "lists no plugins"

# Every entry is a plugin the host will download and execute; each field below
# is something drsg relies on being true.
while IFS=$'\t' read -r name version claims url sha contract min_drsg; do
    where="$name@$version"

    [[ "$name" =~ ^[a-z0-9_-]+$ ]] ||
        bad "$where: name must be lowercase letters, digits, '-' or '_' — it becomes a filename and a config section"
    [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] ||
        bad "$where: version must be X.Y.Z"
    if [ -z "$claims" ] || [ "$claims" = "null" ]; then
        bad "$where: claims is what the installer prints in the table; it cannot be empty"
    fi

    # The URL must name this plugin's artifact at this plugin's tag. A copy-
    # paste that leaves the previous plugin's tag in place is the easy mistake,
    # and it installs the wrong parser under the right name.
    expected="releases/download/$name-v$version/$name.wasm"
    [[ "$url" == *"$expected" ]] ||
        bad "$where: url should end in '$expected', got '$url'"
    [[ "$url" == https://* ]] ||
        bad "$where: url must be https"

    [[ "$sha" =~ ^[0-9a-f]{64}$ ]] ||
        bad "$where: sha256 must be 64 lowercase hex digits"

    [ "$contract" = "$wit_contract" ] ||
        bad "$where: contract is '$contract' but the canonical WIT declares '$wit_contract'"
    [[ "$min_drsg" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] ||
        bad "$where: min_drsg must be X.Y.Z — it is the oldest drsg this entry claims to work with"

    if [ "$online" = 1 ]; then
        published=$(curl -sSfL "$url.sha256" 2>/dev/null | cut -d' ' -f1 || true)
        if [ -z "$published" ]; then
            bad "$where: could not fetch $url.sha256 — is the release published?"
        elif [ "$published" != "$sha" ]; then
            bad "$where: pinned sha256:$sha but the release publishes sha256:$published"
        fi
    fi
done < <(jq -r '.plugins[] | [.name, .version, .claims, .url, .sha256, (.contract // ""), (.min_drsg // "")] | @tsv' "$catalog")

# Two entries with the same name and version are ambiguous: drsg's picker sorts
# by version and would choose between them arbitrarily.
dupes=$(jq -r '.plugins | group_by(.name + "@" + .version) | map(select(length > 1) | .[0].name + "@" + .[0].version) | .[]' "$catalog")
[ -z "$dupes" ] || bad "duplicate entries: $dupes"

if [ "$fail" = 0 ]; then
    if [ "$online" = 1 ]; then
        echo "catalog: $count entries, every artifact hash matches its release"
    else
        echo "catalog: $count entries well-formed (shape only — --online checks the hashes)"
    fi
fi
exit "$fail"

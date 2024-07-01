#!/bin/sh
# SPDX-License-Identifier: GPL-2.0
#
# Print the Rust compiler name and its version in a 5 or 6-digit form.
# Also, perform the minimum version check.

set -e

# Convert the version string x.y.z to a canonical up-to-7-digits form.
#
# Note that this function uses one more digit (compared to other
# instances in other version scripts) to give a bit more space to
# `rustc` since it will reach 1.100.0 in late 2026.
get_canonical_version()
{
	IFS=.
	set -- $1
	echo $((100000 * $1 + 100 * $2 + $3))
}

orig_args="$@"

set -- $("$@" --version)

name=$1

min_tool_version=$(dirname $0)/min-tool-version.sh

case "$name" in
rustc)
	version=$2
	min_version=$($min_tool_version rustc)
	;;
*)
	echo "$orig_args: unknown Rust compiler" >&2
	exit 1
	;;
esac

rustcversion=$(get_canonical_version $version)
min_rustcversion=$(get_canonical_version $min_version)

if [ "$rustcversion" -lt "$min_rustcversion" ]; then
	echo >&2 "***"
	echo >&2 "*** Rust compiler is too old."
	echo >&2 "***   Your $name version:    $version"
	echo >&2 "***   Minimum $name version: $min_version"
	echo >&2 "***"
	exit 1
fi

echo $name $rustcversion

#!/bin/sh
# SPDX-License-Identifier: GPL-2.0
#
# Usage: $ ./rustc-version.sh rustc
#
# Print the Rust compiler version and the LLVM version it uses in a 6 or
# 7-digit form.

# Convert the version string x.y.z to a canonical up-to-6-digits form.
get_llvm_canonical_version()
{
	IFS=.
	set -- $1
	echo $((10000 * $1 + 100 * $2 + $3))
}

# Convert the version string x.y.z to a canonical up-to-7-digits form.
#
# Note that this function uses one more digit (compared to other instances in
# other version scripts and the instance above) to give a bit more space to
# `rustc` since it will reach 1.100.0 in late 2026.
get_rustc_canonical_version()
{
	IFS=.
	set -- $1
	echo $((100000 * $1 + 100 * $2 + $3))
}

if output=$("$@" --version 2>/dev/null); then
	set -- $output
	rustc_version=$(get_rustc_canonical_version $2)
else
	echo 0 0
	exit 1
fi

if output=$("$@" --version --verbose 2>/dev/null | grep LLVM); then
	set -- $output
	rustc_llvm_version=$(get_llvm_canonical_version $3)
else
	echo 0 0
	exit 1
fi

echo $rustc_version $rustc_llvm_version

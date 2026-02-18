#!/bin/bash
set -e

# This script is intended to be run from a Linux kernel checkout.
# It pushes the specified commits to the 'github' remote,
# then updates the 'linux' submodule in the CI repository,
# and finally runs submit_ci.sh.

CI_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Pass-through arguments for submit_ci.sh
PASSTHROUGH=()
POSITIONAL=()

while [[ $# -gt 0 ]]; do
  case $1 in
    -s|--sleep)
      PASSTHROUGH+=("-s" "$2")
      shift 2
      ;;
    -f|--fixes)
      PASSTHROUGH+=("-f" "$2")
      shift 2
      ;;
    *)
      POSITIONAL+=("$1")
      shift
      ;;
  esac
done

if [[ ${#POSITIONAL[@]} -eq 0 ]]; then
  echo "Usage: $0 [-s seconds] [-f fixes_branch] [base] tip"
  exit 1
fi

if [[ ${#POSITIONAL[@]} -eq 1 ]]; then
  BASE=""
  TIP="${POSITIONAL[0]}"
else
  BASE="${POSITIONAL[0]}"
  TIP="${POSITIONAL[1]}"
fi

TIP_HASH=$(git rev-parse "$TIP")
BASE_HASH=""
if [[ -n "$BASE" ]]; then
  BASE_HASH=$(git rev-parse "$BASE")
fi

echo "Pushing $TIP ($TIP_HASH) to github:ci/incoming..."
git push --force github "$TIP_HASH:refs/heads/ci/incoming"

echo "Updating CI submodule..."
(
  cd "$CI_DIR/linux"
  git fetch origin ci/incoming
)

cd "$CI_DIR"
if [[ -n "$BASE_HASH" ]]; then
  ./submit_ci.sh "${PASSTHROUGH[@]}" "$BASE_HASH" "$TIP_HASH"
else
  ./submit_ci.sh "${PASSTHROUGH[@]}" "$TIP_HASH"
fi

#!/bin/bash
set -e

SLEEP_SEC=""
while [[ $# -gt 0 ]]; do
  case $1 in
    -s|--sleep)
      SLEEP_SEC="$2"
      shift 2
      ;;
    *)
      break
      ;;
  esac
done

if [[ $# -lt 2 ]]; then
  echo "Usage: $0 [-s seconds] <base-commit> <tip-commit>"
  echo "Example: $0 origin/master b4/driver-types"
  exit 1
fi

BASE_COMMIT="$1"
TIP_COMMIT="$2"

# Fix commits to apply (hardcoded for now)
# 780b05710047: rust_binder: Fix build failure if !CONFIG_COMPAT
# c658b7542cb9: rust: bitops: fix missing _find_* functions on 32-bit ARM
FIXES="780b05710047 c658b7542cb9"

# Prepare Fixes Branch
echo "Preparing fixes branch based on $BASE_COMMIT..."
(
  cd linux
  # Create/Reset ci/base-fixes branch
  git checkout -B ci/base-fixes "$BASE_COMMIT" > /dev/null 2>&1
  for FIX in $FIXES; do
    git cherry-pick "$FIX" > /dev/null
  done
)

# Get list of commits to test (oldest to newest)
echo "Generating list of commits between $BASE_COMMIT and $TIP_COMMIT..."
COMMITS=$(cd linux && git rev-list --reverse "${BASE_COMMIT}..${TIP_COMMIT}")

if [[ -z "$COMMITS" ]]; then
  echo "No commits found in range."
  exit 0
fi

echo "Found $(echo "$COMMITS" | wc -l) commits to test."

for COMMIT in $COMMITS; do
  SHORT_COMMIT=$(echo "$COMMIT" | cut -c1-12)
  echo "========================================"
  echo "Processing submodule commit $SHORT_COMMIT"
  echo "========================================"
  
  # 1. Prepare Submodule
  echo "Preparing submodule..."
  (
    cd linux
    git checkout --detach "$COMMIT" > /dev/null 2>&1
    # Merge fixes
    git merge --no-edit ci/base-fixes > /dev/null
    # Push to a stable ref for the submodule
    git push --force origin HEAD:refs/heads/ci/fixes
  )

  # 2. Update Parent
  echo "Updating parent repository..."
  git add linux
  # Amend the previous commit to avoid creating a huge history in the parent if running repeatedly? 
  # But we want to test each one.
  git commit -m "ci: Update submodule to $SHORT_COMMIT (testing)"

  # 3. Push Parent
  echo "Pushing to CI..."
  git push --force origin ci/actions

  # 4. Wait
  if [[ -n "$SLEEP_SEC" ]]; then
    echo "Sleeping for $SLEEP_SEC seconds..."
    sleep "$SLEEP_SEC"
  else
    echo "Check GitHub Actions: https://github.com/Darksonn/linux/actions"
    read -p "Press Enter when the CI job has started to proceed to the next commit..."
  fi
done

echo "Done!"
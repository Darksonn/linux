#!/bin/bash
set -e

FIXES_REF=""
SLEEP_SEC=""
while [[ $# -gt 0 ]]; do
  case $1 in
    -s|--sleep)
      SLEEP_SEC="$2"
      shift 2
      ;;
    -f|--fixes)
      FIXES_REF="$2"
      shift 2
      ;;
    *)
      break
      ;;
  esac
done

if [[ $# -eq 0 ]]; then
  echo "Usage: $0 [-s seconds] [-f fixes_branch] <commit-or-range-start> [range-end]"
  echo "Example (single): $0 b4/driver-types"
  echo "Example (range):  $0 origin/master b4/driver-types"
  exit 1
fi

# Resolve Fixes Branch
if [[ -n "$FIXES_REF" ]]; then
  echo "Checking for custom fixes branch '$FIXES_REF'..."
  if ! (cd linux && git rev-parse --verify "$FIXES_REF" >/dev/null 2>&1); then
    echo "Error: local branch '$FIXES_REF' not found in linux submodule."
    exit 1
  fi
else
  echo "Checking for default fixes branch 'ci/base-fixes'..."
  if (cd linux && git rev-parse --verify ci/base-fixes >/dev/null 2>&1); then
    FIXES_REF="ci/base-fixes"
    echo "Found local 'ci/base-fixes', using it."
  else
    echo "Local 'ci/base-fixes' not found. Proceeding without merging fixes."
  fi
fi

if [[ $# -eq 1 ]]; then
  TIP_COMMIT="$1"
  # Resolve to a full hash to be consistent
  COMMITS=$(cd linux && git rev-parse "$TIP_COMMIT")
  echo "Testing single commit: $TIP_COMMIT ($COMMITS)"
else
  BASE_COMMIT="$1"
  TIP_COMMIT="$2"
  # Get list of commits to test (oldest to newest)
  echo "Generating list of commits between $BASE_COMMIT and $TIP_COMMIT..."
  COMMITS=$(cd linux && git rev-list --reverse "${BASE_COMMIT}..${TIP_COMMIT}")
fi

if [[ -z "$COMMITS" ]]; then
  echo "No commits found to test."
  exit 0
fi

TOTAL_COMMITS=$(echo "$COMMITS" | wc -l)
echo "Found $TOTAL_COMMITS commits to test."

COUNT=0
for COMMIT in $COMMITS; do
  COUNT=$((COUNT + 1))
  SHORT_COMMIT=$(echo "$COMMIT" | cut -c1-12)
  COMMIT_SUBJECT=$(cd linux && git show -s --format=%s "$COMMIT")
  echo "========================================"
  echo "Processing submodule commit $SHORT_COMMIT: $COMMIT_SUBJECT"
  echo "========================================"
  
  # 1. Prepare Submodule
  echo "Preparing submodule..."
  (
    cd linux
    git checkout --detach "$COMMIT"
    # Merge fixes if defined
    if [[ -n "$FIXES_REF" ]]; then
      git merge --no-edit "$FIXES_REF"
    fi
    # Push to a stable ref for the submodule
    git push --force origin HEAD:refs/heads/ci/fixes
  )

  # 2. Update Parent
  echo "Updating parent repository..."
  git add linux
  # Amend the previous commit to avoid creating a huge history in the parent if running repeatedly? 
  # The user said "final history in the parent repository... is linear".
  # If we just keep making new commits, we get a linear history.
  git commit -m "$SHORT_COMMIT: $COMMIT_SUBJECT"

  # 3. Push Parent
  echo "Pushing to CI..."
  git push --force origin ci/actions

  # 4. Wait
  if [[ $COUNT -lt $TOTAL_COMMITS ]]; then
    if [[ -n "$SLEEP_SEC" ]]; then
      echo "Sleeping for $SLEEP_SEC seconds..."
      sleep "$SLEEP_SEC"
    else
      echo "Check GitHub Actions: https://github.com/Darksonn/linux/actions"
      read -p "Press Enter when the CI job has started to proceed to the next commit..."
    fi
  fi
done

echo "Done!"

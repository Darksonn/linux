# Rust for Linux CI

This repository hosts a Continuous Integration (CI) configuration for testing Rust patches for the Linux kernel.

## Usage

To submit a series of commits for testing on GitHub Actions, use the `submit_ci.sh` script.

### Prerequisites

1.  **Submodule Setup:** Ensure the `linux` submodule is initialized and updated.
    *   The submodule must have a remote named `origin` that you have permissions to push to.
2.  **Parent Repo Setup:** The parent repository must also have a remote named `origin` that you have permissions to push to.
3.  **Pull Request:** You must have an open Pull Request in this repository targeting your submission branch (typically from `ci/actions`). This is necessary because the CI workflow is triggered on `pull_request` events.
4.  **Fixes Branch (Optional):** If you have a local branch named `ci/base-fixes` in the `linux` submodule, the script will automatically merge it into every commit you test.
    *   This is useful for applying temporary fixes or backports required for the CI to pass.
    *   You can specify a different branch name using the `-f` flag.

### Running Tests

Run the script with either a single commit or a range of commits (start and end points in the submodule):

```bash
./submit_ci.sh [target-commit]
./submit_ci.sh [base-commit] [tip-commit]
```

Example (single commit):
```bash
./submit_ci.sh b4/driver-types
```

Example (range):
```bash
./submit_ci.sh origin/master b4/driver-types
```

**Options:**
*   `-s <seconds>`: Sleep for the specified number of seconds between pushing commits (useful for pacing CI runs). Default behavior is interactive (wait for Enter).
*   `-f <branch>`: Specify a custom fixes branch to merge into each commit (defaults to `ci/base-fixes` if it exists).

### How it works

For each commit in the specified range:
1.  Checks out the commit in the `linux` submodule.
2.  Merges `ci/base-fixes` into it.
3.  Pushes the result to the `ci/fixes` branch in the submodule's remote.
4.  Updates the parent repository to point to this new submodule state.
5.  Pushes the parent repository to the `ci/actions` branch, triggering the GitHub Actions workflow.

## Workflows

The CI pipeline (`.github/workflows/ci.yml`) performs:
*   **Build:** Builds the kernel with `LLVM=1` and `CLIPPY=1` for `x86_64`, `arm64`, `riscv`, etc.
*   **Test:** Runs KUnit tests under QEMU for `x86_64` and `arm64`.
*   **Selftests:** Builds x86_64 kernel and modules, generates an on-demand Debian disk image, and runs userspace kernel selftests (including Rust sample module probe tests, binderfs, pidfd, and memfd suites) under QEMU.
*   **Checkpatch:** Runs `scripts/checkpatch.pl` on the commit.
*   **Rustfmt:** Checks code formatting using `make rustfmtcheck`.

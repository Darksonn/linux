# Rust for Linux CI

This repository hosts the Continuous Integration (CI) configuration for the [Rust for Linux](https://github.com/Rust-for-Linux/linux) project. It runs build tests, checkpatch, KUnit tests (via QEMU), and formatting checks.

## Usage

To submit a series of commits for testing on GitHub Actions, use the `submit_ci.sh` script.

### Prerequisites

1.  **Submodule Setup:** Ensure the `linux` submodule is initialized and updated.
2.  **Fixes Branch (Optional):** If you have a local branch named `ci/base-fixes` in the `linux` submodule, the script will automatically merge it into every commit you test.
    *   This is useful for applying temporary fixes or backports required for the CI to pass.
    *   You can specify a different branch name using the `-f` flag.

### Running Tests

Run the script with either a single commit or a range of commits (start and end points in the submodule):

```bash
./submit_ci.sh <commit-or-range-start> [range-end]
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
*   **Checkpatch:** Runs `scripts/checkpatch.pl` on the commit (checking the first parent if it's a merge).
*   **Rustfmt:** Checks code formatting using `make rustfmtcheck`.

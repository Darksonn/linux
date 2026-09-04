#!/usr/bin/env bash
# run_vm_tests.sh - Test suite runner executed inside the Debian VM
set -euo pipefail

echo "=========================================="
echo "Running selftests inside Debian QEMU guest"
echo "Kernel release: $(uname -r)"
echo "Architecture:   $(uname -m)"
echo "=========================================="

FAILED=0

run_test() {
    local name="$1"
    local test_bin="$2"
    shift 2
    local dir
    dir="$(dirname "$test_bin")"
    echo "=========================================="
    echo "=== Running test: $name ==="
    echo "=========================================="
    set +e
    (cd "$dir" && "$test_bin" "$@")
    local rc=$?
    set -e
    if [ $rc -eq 0 ]; then
        echo "=== [PASS] $name ==="
    else
        echo "=== [FAIL] $name (exit code: $rc) ==="
        FAILED=1
    fi
}

# 1. Rust sample module selftests
if [ -x /mnt/linux/tools/testing/selftests/rust/test_probe_samples.sh ]; then
    run_test "rust: test_probe_samples.sh" /mnt/linux/tools/testing/selftests/rust/test_probe_samples.sh
else
    echo "Warning: /mnt/linux/tools/testing/selftests/rust/test_probe_samples.sh not found or not executable"
fi

# 2. Binderfs selftests
modprobe -q rust_binder 2>/dev/null || true
if [ -x /mnt/linux/tools/testing/selftests/filesystems/binderfs/binderfs_test ]; then
    run_test "filesystems/binderfs: binderfs_test" /mnt/linux/tools/testing/selftests/filesystems/binderfs/binderfs_test
else
    echo "Warning: /mnt/linux/tools/testing/selftests/filesystems/binderfs/binderfs_test not found"
fi

# 3. Pidfd selftests
if [ -d /mnt/linux/tools/testing/selftests/pidfd ]; then
    for test_bin in /mnt/linux/tools/testing/selftests/pidfd/pidfd_*; do
        if [ -x "$test_bin" ] && [ "$(basename "$test_bin")" != "pidfd_exec_helper" ]; then
            run_test "pidfd: $(basename "$test_bin")" "$test_bin"
        fi
    done
fi

# 4. Memfd selftests
if [ -x /mnt/linux/tools/testing/selftests/memfd/memfd_test ]; then
    run_test "memfd: memfd_test" /mnt/linux/tools/testing/selftests/memfd/memfd_test
fi

echo "=========================================="
if [ "$FAILED" -eq 0 ]; then
    echo "All VM selftests passed successfully!"
    exit 0
else
    echo "One or more VM selftests failed!"
    exit 1
fi


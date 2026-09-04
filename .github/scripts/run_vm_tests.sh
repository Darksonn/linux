#!/usr/bin/env bash
# run_vm_tests.sh - Test suite runner executed inside the Debian VM
set -euo pipefail

echo "=========================================="
echo "Running selftests inside Debian QEMU guest"
echo "Kernel release: $(uname -r)"
echo "Architecture:   $(uname -m)"
echo "=========================================="

# 1. Run Rust sample module selftests
echo "=== [1/1] Running Rust sample module selftests ==="
if [ -d /mnt/linux/tools/testing/selftests/rust ]; then
    cd /mnt/linux/tools/testing/selftests/rust
    ./test_probe_samples.sh
else
    echo "Error: /mnt/linux/tools/testing/selftests/rust not found!"
    exit 1
fi

echo "=========================================="
echo "All VM selftests passed successfully!"
echo "=========================================="

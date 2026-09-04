#!/usr/bin/env bash
# run_qemu.sh - Boots QEMU with Debian disk image and runs kernel selftests
set -euo pipefail

IMG="${1:-debian.img}"
KERNEL="${2:-out/arch/x86/boot/bzImage}"
RESULT_FILE="test_result.txt"

rm -f "$RESULT_FILE"

if [ ! -f "$IMG" ]; then
    echo "Error: Disk image not found at $IMG"
    exit 1
fi

if [ ! -f "$KERNEL" ]; then
    echo "Error: Kernel bzImage not found at $KERNEL"
    exit 1
fi

echo "Booting QEMU VM with $KERNEL and $IMG..."

# Run QEMU with a 10-minute timeout to guard against hangs.
# Use KVM acceleration if available, fall back to TCG.
set +e
timeout 600 qemu-system-x86_64 \
    -machine q35,acpi=on \
    -accel kvm:tcg \
    -kernel "$KERNEL" \
    -drive "file=$IMG,format=raw,if=virtio" \
    -append "root=/dev/vda console=ttyS0 acpi=force panic=-1" \
    -nographic \
    -no-reboot \
    -m 2G -smp 2 \
    -virtfs "local,path=$PWD,mount_tag=hostshare,security_model=none,id=hostshare"
QEMU_EXIT=$?
set -e

echo "QEMU process exited with code: $QEMU_EXIT"

if [ ! -f "$RESULT_FILE" ]; then
    echo "Error: Test result file ($RESULT_FILE) was not written! (VM crashed, panicked, or timed out)"
    exit 1
fi

RESULT=$(cat "$RESULT_FILE" | tr -d '[:space:]')
echo "VM selftest result exit code: $RESULT"

if [ "$RESULT" -ne 0 ]; then
    echo "Selftests failed inside VM with exit code $RESULT"
    exit "$RESULT"
fi

echo "Selftests completed successfully!"

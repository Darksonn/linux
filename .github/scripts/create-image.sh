#!/usr/bin/env bash
# create-image.sh - Creates a minimal Debian disk image for QEMU kernel selftests
set -euo pipefail

IMG="${1:-debian.img}"
MODULES_DIR="${2:-}"
DISTRO="bookworm"
DIR=$(mktemp -d /tmp/debian-mount-XXXXXX)

cleanup() {
    echo "Cleaning up mount directory $DIR..."
    if mountpoint -q "$DIR"; then
        umount "$DIR" || umount -l "$DIR" || true
    fi
    rm -rf "$DIR"
}
trap cleanup EXIT

echo "Creating 2GB ext4 disk image: $IMG"
dd if=/dev/zero of="$IMG" bs=1M seek=2047 count=1
mkfs.ext4 -F "$IMG"

echo "Mounting $IMG at $DIR..."
mount -o loop "$IMG" "$DIR"

echo "Installing minimal Debian ($DISTRO) with debootstrap..."
debootstrap --arch=amd64 \
    --include=kmod,udev,procps \
    "$DISTRO" "$DIR" http://deb.debian.org/debian/

echo "Configuring hostname and fstab..."
echo "debian-vm" > "$DIR/etc/hostname"
mkdir -p "$DIR/mnt"
cat <<EOF > "$DIR/etc/fstab"
/dev/root / ext4 defaults 0 0
hostshare /mnt 9p trans=virtio,version=9p2000.L,nofail 0 0
EOF

# Allow passwordless root login
sed -i 's/^root:[^:]*:/root::/' "$DIR/etc/shadow"

# Install kernel modules into image if provided
if [ -n "$MODULES_DIR" ] && [ -d "$MODULES_DIR/lib/modules" ]; then
    echo "Installing kernel modules from $MODULES_DIR into image..."
    mkdir -p "$DIR/lib/modules"
    cp -a "$MODULES_DIR/lib/modules"/* "$DIR/lib/modules/"
    for kver_dir in "$DIR/lib/modules"/*; do
        if [ -d "$kver_dir" ]; then
            kver=$(basename "$kver_dir")
            echo "Running depmod for $kver inside image..."
            chroot "$DIR" depmod -a "$kver" || true
        fi
    done
fi

# Set up systemd service to run selftests on boot
echo "Setting up run-selftests systemd service..."
cat <<'EOF' > "$DIR/etc/systemd/system/run-selftests.service"
[Unit]
Description=Run Kernel Selftests in VM
After=local-fs.target
Requires=local-fs.target

[Service]
Type=oneshot
ExecStart=/root/run_selftests.sh
StandardInput=null
StandardOutput=journal+console
StandardError=journal+console

[Install]
WantedBy=multi-user.target
EOF

mkdir -p "$DIR/etc/systemd/system/multi-user.target.wants"
ln -sf /etc/systemd/system/run-selftests.service "$DIR/etc/systemd/system/multi-user.target.wants/run-selftests.service"

# Create /root/run_selftests.sh inside the VM image
cat <<'EOF' > "$DIR/root/run_selftests.sh"
#!/bin/bash
exec > /dev/console 2>&1
echo "========================================"
echo "=== Booted Debian VM for Kernel CI   ==="
echo "========================================"

# Mount /mnt from host if not already mounted
if ! mountpoint -q /mnt; then
    echo "Mounting hostshare on /mnt..."
    mount -t 9p -o trans=virtio,version=9p2000.L hostshare /mnt || true
fi

EXIT_CODE=1
if [ -x /mnt/.github/scripts/run_vm_tests.sh ]; then
    echo "=== Executing /mnt/.github/scripts/run_vm_tests.sh ==="
    set +e
    /mnt/.github/scripts/run_vm_tests.sh
    EXIT_CODE=$?
    set -e
else
    echo "Error: /mnt/.github/scripts/run_vm_tests.sh not found or not executable!"
    EXIT_CODE=2
fi

echo "=== Tests completed with exit code: $EXIT_CODE ==="
echo "$EXIT_CODE" > /mnt/test_result.txt
chmod 666 /mnt/test_result.txt 2>/dev/null || true
sync

echo "=== Powering off VM ==="
poweroff -f
EOF
chmod +x "$DIR/root/run_selftests.sh"

echo "Unmounting image..."
umount "$DIR"

# Restore ownership to caller user if running under sudo
if [ -n "${SUDO_UID:-}" ] && [ -n "${SUDO_GID:-}" ]; then
    chown "$SUDO_UID:$SUDO_GID" "$IMG"
elif [ -n "${SUDO_USER:-}" ]; then
    chown "$SUDO_USER" "$IMG"
fi

echo "Success! Created $IMG ready for QEMU."

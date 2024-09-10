/* SPDX-License-Identifier: GPL-2.0 */

#ifndef __ASM_GENERIC_SPINLOCK_TYPES_H
#define __ASM_GENERIC_SPINLOCK_TYPES_H

#include <linux/types.h>
typedef atomic_t arch_spinlock_t;

/*
 * qrwlock_types depends on arch_spinlock_t, so we must typedef that before the
 * include.
 */
#include <asm/qrwlock_types.h>

#define __ARCH_SPIN_LOCK_UNLOCKED	ATOMIC_INIT(0)

/*
 * These are used to export the value of __ARCH_SPIN_LOCK_UNLOCKED to Rust. The
 * type must have the same size as arch_spinlock_t, and the value must be
 * represented using the same sequence of bytes as __ARCH_SPIN_LOCK_UNLOCKED.
 *
 * Due to limitations in bindgen, the type must be one of the integer types,
 * and can't be a struct or atomic_t.
 */
#define __ARCH_SPIN_LOCK_UNLOCKED_TYP	int
#define __ARCH_SPIN_LOCK_UNLOCKED_INT	0

#endif /* __ASM_GENERIC_SPINLOCK_TYPES_H */

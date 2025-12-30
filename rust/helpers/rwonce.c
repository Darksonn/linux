// SPDX-License-Identifier: GPL-2.0

/*
 * Copyright (C) 2025 Google LLC.
 */

#ifdef CONFIG_ARCH_USE_CUSTOM_READ_ONCE

__rust_helper u8 rust_helper_read_once_1(const u8 *ptr)
{
	return READ_ONCE(*ptr);
}

__rust_helper u16 rust_helper_read_once_2(const u16 *ptr)
{
	return READ_ONCE(*ptr);
}

__rust_helper u32 rust_helper_read_once_4(const u32 *ptr)
{
	return READ_ONCE(*ptr);
}

__rust_helper u64 rust_helper_read_once_8(const u64 *ptr)
{
	return READ_ONCE(*ptr);
}

__rust_helper void *rust_helper_read_once_ptr(void * const *ptr)
{
	return READ_ONCE(*ptr);
}

#endif

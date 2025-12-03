// SPDX-License-Identifier: GPL-2.0

#include <linux/bitops.h>
#include <linux/find.h>

void rust_helper___set_bit(unsigned long nr, unsigned long *addr)
{
	__set_bit(nr, addr);
}

void rust_helper___clear_bit(unsigned long nr, unsigned long *addr)
{
	__clear_bit(nr, addr);
}

void rust_helper_set_bit(unsigned long nr, volatile unsigned long *addr)
{
	set_bit(nr, addr);
}

void rust_helper_clear_bit(unsigned long nr, volatile unsigned long *addr)
{
	clear_bit(nr, addr);
}

/*
 * Rust normally calls the single-underscore-prefixed version of these
 * functions, which are not inlined. However, on some platforms, they do not
 * exist. In those cases, provide a rust helper for the underscored version.
 */
#ifdef find_next_zero_bit
__rust_helper unsigned long
rust_helper__find_next_zero_bit(const unsigned long *addr, unsigned long size,
				unsigned long offset)
{
	return find_next_zero_bit(addr, size, offset);
}
#endif /* find_next_zero_bit */

#ifdef find_next_bit
__rust_helper unsigned long
rust_helper__find_next_bit(const unsigned long *addr, unsigned long size,
			   unsigned long offset)
{
	return find_next_bit(addr, size, offset);
}
#endif /* find_next_bit */

#ifdef find_last_bit
__rust_helper unsigned long
rust_helper__find_last_bit(const unsigned long *addr, unsigned long size)
{
	return find_last_bit(addr, size);
}
#endif /* find_last_bit */

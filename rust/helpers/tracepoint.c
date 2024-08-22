// SPDX-License-Identifier: GPL-2.0

/*
 * Helpers for tracepoints. At the moment, helpers are only needed when
 * CONFIG_JUMP_LABEL is disabled, as `static_key_count` is only marked inline
 * in that case.
 *
 * Copyright (C) 2024 Google LLC.
 */

#include <linux/jump_label.h>

#ifndef CONFIG_JUMP_LABEL
int rust_helper_static_key_count(struct static_key *key)
{
	return static_key_count(key);
}
#endif

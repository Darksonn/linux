// SPDX-License-Identifier: GPL-2.0

#include <linux/security.h>

#ifndef CONFIG_SECURITY
int rust_helper_security_secid_to_secctx(u32 secid, struct lsm_context *cp)
{
	return security_secid_to_secctx(secid, cp);
}

void rust_helper_security_release_secctx(struct lsm_context *cp)
{
	security_release_secctx(cp);
}
#endif

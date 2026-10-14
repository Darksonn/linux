// SPDX-License-Identifier: GPL-2.0

#include <linux/gfp.h>
#include <linux/highmem.h>
#include <linux/memcontrol.h>
#include <linux/mm.h>

__rust_helper struct page *rust_helper_alloc_pages(gfp_t gfp_mask,
						   unsigned int order)
{
	return alloc_pages(gfp_mask, order);
}

__rust_helper void rust_helper_put_page(struct page *page)
{
	put_page(page);
}

__rust_helper struct folio *rust_helper_page_folio(struct page *page)
{
	return page_folio(page);
}

__rust_helper int rust_helper_mem_cgroup_charge(struct folio *folio,
						struct mm_struct *mm, gfp_t gfp)
{
	return mem_cgroup_charge(folio, mm, gfp);
}

__rust_helper void *rust_helper_kmap_local_page(struct page *page)
{
	return kmap_local_page(page);
}

__rust_helper void rust_helper_kunmap_local(const void *addr)
{
	kunmap_local(addr);
}

#ifndef NODE_NOT_IN_PAGE_FLAGS
__rust_helper int rust_helper_page_to_nid(const struct page *page)
{
	return page_to_nid(page);
}
#endif

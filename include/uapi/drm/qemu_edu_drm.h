/* SPDX-License-Identifier: GPL-2.0 WITH Linux-syscall-note */
#ifndef _UAPI_QEMU_EDU_DRM_H_
#define _UAPI_QEMU_EDU_DRM_H_

#include "drm.h"

#if defined(__cplusplus)
extern "C" {
#endif

struct drm_edu_get_id {
	__u32 id;
};

struct drm_edu_test_liveness {
	__u32 val;
	__u32 inv;
};

struct drm_edu_compute_factorial {
	__u32 val;
	__u32 res;
};

struct drm_edu_test_irq {
	__u32 val;
};

#define DRM_EDU_GET_ID             0x00
#define DRM_EDU_TEST_LIVENESS      0x01
#define DRM_EDU_COMPUTE_FACTORIAL  0x02
#define DRM_EDU_TEST_IRQ           0x03

enum {
	DRM_IOCTL_EDU_GET_ID            = DRM_IOR(DRM_COMMAND_BASE + DRM_EDU_GET_ID, struct drm_edu_get_id),
	DRM_IOCTL_EDU_TEST_LIVENESS     = DRM_IOWR(DRM_COMMAND_BASE + DRM_EDU_TEST_LIVENESS, struct drm_edu_test_liveness),
	DRM_IOCTL_EDU_COMPUTE_FACTORIAL = DRM_IOWR(DRM_COMMAND_BASE + DRM_EDU_COMPUTE_FACTORIAL, struct drm_edu_compute_factorial),
	DRM_IOCTL_EDU_TEST_IRQ          = DRM_IOW(DRM_COMMAND_BASE + DRM_EDU_TEST_IRQ, struct drm_edu_test_irq),
};

#if defined(__cplusplus)
}
#endif

#endif /* _UAPI_QEMU_EDU_DRM_H_ */

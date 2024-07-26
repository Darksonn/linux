// SPDX-License-Identifier: GPL-2.0 or MIT
/* Copyright 2019 Collabora ltd. */

#include <linux/clk.h>
#include <linux/devfreq.h>
#include <linux/devfreq_cooling.h>
#include <linux/platform_device.h>
#include <linux/pm_opp.h>

#include <drm/drm_managed.h>

#include "panthor_devfreq.h"
#include "panthor_device.h"

int panthor_devfreq_init(struct panthor_device *ptdev)
{
	/* There's actually 2 regulators (mali and sram), but the OPP core only
	 * supports one.
	 *
	 * We assume the sram regulator is coupled with the mali one and let
	 * the coupling logic deal with voltage updates.
	 */
	static const char * const reg_names[] = { "mali", NULL };
	struct device *dev = ptdev->base.dev;
	struct panthor_devfreq *pdevfreq;
	struct dev_pm_opp *opp;
	unsigned long cur_freq;
	int ret;

	pdevfreq = drmm_kzalloc(&ptdev->base, PANTHOR_DEVFREQ_SIZEOF, GFP_KERNEL);
	if (!pdevfreq)
		return -ENOMEM;

	ret = devm_pm_opp_set_regulators(dev, reg_names);
	if (ret) {
		if (ret != -EPROBE_DEFER)
			DRM_DEV_ERROR(dev, "Couldn't set OPP regulators\n");

		return ret;
	}

	ret = devm_pm_opp_of_add_table(dev);
	if (ret)
		return ret;

	cur_freq = clk_get_rate(ptdev->clks.core);

	opp = devfreq_recommended_opp(dev, &cur_freq, 0);
	if (IS_ERR(opp))
		return PTR_ERR(opp);

	/* Regulator coupling only takes care of synchronizing/balancing voltage
	 * updates, but the coupled regulator needs to be enabled manually.
	 *
	 * We use devm_regulator_get_enable_optional() and keep the sram supply
	 * enabled until the device is removed, just like we do for the mali
	 * supply, which is enabled when dev_pm_opp_set_opp(dev, opp) is called,
	 * and disabled when the opp_table is torn down, using the devm action.
	 *
	 * If we really care about disabling regulators on suspend, we should:
	 * - use devm_regulator_get_optional() here
	 * - call dev_pm_opp_set_opp(dev, NULL) before leaving this function
	 *   (this disables the regulator passed to the OPP layer)
	 * - call dev_pm_opp_set_opp(dev, NULL) and
	 *   regulator_disable(ptdev->regulators.sram) in
	 *   panthor_devfreq_suspend()
	 * - call dev_pm_opp_set_opp(dev, default_opp) and
	 *   regulator_enable(ptdev->regulators.sram) in
	 *   panthor_devfreq_resume()
	 *
	 * But without knowing if it's beneficial or not (in term of power
	 * consumption), or how much it slows down the suspend/resume steps,
	 * let's just keep regulators enabled for the device lifetime.
	 */
	ret = devm_regulator_get_enable_optional(dev, "sram");
	if (ret && ret != -ENODEV) {
		if (ret != -EPROBE_DEFER)
			DRM_DEV_ERROR(dev, "Couldn't retrieve/enable sram supply\n");
		return ret;
	}

	/*
	 * Set the recommend OPP this will enable and configure the regulator
	 * if any and will avoid a switch off by regulator_late_cleanup()
	 */
	ret = dev_pm_opp_set_opp(dev, opp);
	if (ret) {
		DRM_DEV_ERROR(dev, "Couldn't set recommended OPP\n");
		return ret;
	}

	dev_pm_opp_put(opp);

	ret = panthor_devfreq_init_rust(pdevfreq, ptdev, cur_freq);
	if (ret) {
		DRM_DEV_ERROR(dev, "Couldn't initialize GPU devfreq\n");
		return ret;
	}

	ptdev->devfreq = pdevfreq;

	if (panthor_devfreq_cooling_register(pdevfreq))
		DRM_DEV_INFO(dev, "Failed to register cooling device\n");

	return 0;
}

// SPDX-License-Identifier: GPL-2.0
#![allow(unused_variables, unused_imports)]
//! Rust PCI EDU driver sample with a DRM class device interface and IRQ support.
//!
//! To use this driver:
//!
//! ```c
//! #include <fcntl.h>
//! #include <stdio.h>
//! #include <stdlib.h>
//! #include <string.h>
//! #include <sys/ioctl.h>
//! #include <unistd.h>
//! #include <drm/drm.h>
//!
//! struct drm_edu_get_id {
//!     __u32 id;
//! };
//!
//! struct drm_edu_test_liveness {
//!     __u32 val;
//!     __u32 inv;
//! };
//!
//! struct drm_edu_compute_factorial {
//!     __u32 val;
//!     __u32 res;
//! };
//!
//! struct drm_edu_test_irq {
//!     __u32 val;
//! };
//!
//! #define DRM_EDU_GET_ID             0x00
//! #define DRM_EDU_TEST_LIVENESS      0x01
//! #define DRM_EDU_COMPUTE_FACTORIAL  0x02
//! #define DRM_EDU_TEST_IRQ           0x03
//!
//! #define DRM_IOCTL_EDU_GET_ID            DRM_IOR(DRM_COMMAND_BASE + DRM_EDU_GET_ID, struct drm_edu_get_id)
//! #define DRM_IOCTL_EDU_TEST_LIVENESS     DRM_IOWR(DRM_COMMAND_BASE + DRM_EDU_TEST_LIVENESS, struct drm_edu_test_liveness)
//! #define DRM_IOCTL_EDU_COMPUTE_FACTORIAL DRM_IOWR(DRM_COMMAND_BASE + DRM_EDU_COMPUTE_FACTORIAL, struct drm_edu_compute_factorial)
//! #define DRM_IOCTL_EDU_TEST_IRQ          DRM_IOW(DRM_COMMAND_BASE + DRM_EDU_TEST_IRQ, struct drm_edu_test_irq)
//!
//! void print_usage(const char *prog) {
//!     fprintf(stderr, "Usage:\n");
//!     fprintf(stderr, "  %s id              - Get device ID\n", prog);
//!     fprintf(stderr, "  %s live <value>    - Test liveness (writes value, expects ~value)\n", prog);
//!     fprintf(stderr, "  %s fact <value>    - Compute factorial of value\n", prog);
//!     fprintf(stderr, "  %s irq <value>     - Trigger interrupt with value\n", prog);
//! }
//!
//! int main(int argc, char *argv[]) {
//!     if (argc < 2) {
//!         print_usage(argv[0]);
//!         return 1;
//!     }
//!
//!     int fd = open("/dev/dri/renderD128", O_RDWR);
//!     if (fd < 0) {
//!         perror("Failed to open /dev/dri/renderD128");
//!         return 1;
//!     }
//!
//!     const char *cmd = argv[1];
//!
//!     if (strcmp(cmd, "id") == 0) {
//!         struct drm_edu_get_id arg = {0};
//!         if (ioctl(fd, DRM_IOCTL_EDU_GET_ID, &arg) < 0) {
//!             perror("GET_ID failed");
//!             close(fd);
//!             return 1;
//!         }
//!         printf("Device ID: 0x%08x\n", arg.id);
//!     } else if (strcmp(cmd, "live") == 0) {
//!         if (argc < 3) {
//!             fprintf(stderr, "Error: 'live' requires an integer argument.\n");
//!             print_usage(argv[0]);
//!             close(fd);
//!             return 1;
//!         }
//!         unsigned int val = strtoul(argv[2], NULL, 0);
//!         struct drm_edu_test_liveness arg = { .val = val };
//!         if (ioctl(fd, DRM_IOCTL_EDU_TEST_LIVENESS, &arg) < 0) {
//!             perror("LIVENESS failed");
//!             close(fd);
//!             return 1;
//!         }
//!         printf("Liveness: written=0x%08x, read=0x%08x (expected: 0x%08x)\n",
//!                val, arg.inv, ~val);
//!     } else if (strcmp(cmd, "fact") == 0) {
//!         if (argc < 3) {
//!             fprintf(stderr, "Error: 'fact' requires an integer argument.\n");
//!             print_usage(argv[0]);
//!             close(fd);
//!             return 1;
//!         }
//!         unsigned int val = strtoul(argv[2], NULL, 0);
//!         struct drm_edu_compute_factorial arg = { .val = val };
//!         if (ioctl(fd, DRM_IOCTL_EDU_COMPUTE_FACTORIAL, &arg) < 0) {
//!             perror("FACTORIAL failed");
//!             close(fd);
//!             return 1;
//!         }
//!         printf("Factorial: %u! = %u\n", val, arg.res);
//!     } else if (strcmp(cmd, "irq") == 0) {
//!         if (argc < 3) {
//!             fprintf(stderr, "Error: 'irq' requires an integer argument.\n");
//!             print_usage(argv[0]);
//!             close(fd);
//!             return 1;
//!         }
//!         unsigned int val = strtoul(argv[2], NULL, 0);
//!         struct drm_edu_test_irq arg = { .val = val };
//!         if (ioctl(fd, DRM_IOCTL_EDU_TEST_IRQ, &arg) < 0) {
//!             perror("IRQ failed");
//!             close(fd);
//!             return 1;
//!         }
//!         printf("IRQ triggered with value %u. Check dmesg for handled log.\n", val);
//!     } else {
//!         fprintf(stderr, "Error: Unknown command '%s'\n", cmd);
//!         print_usage(argv[0]);
//!         close(fd);
//!         return 1;
//!     }
//!
//!     close(fd);
//!     return 0;
//! }
//! ````

use kernel::{
    device::{Bound, Core, DeviceContext},
    drm,
    drm::ioctl,
    drm::Registered,
    io::{poll, register, Io},
    irq, pci,
    prelude::*,
    sync::aref::ARef,
    time, uapi,
};

mod regs {
    use super::*;
    register! {
        pub(super) ID(u32) @ 0x00 {
            31:0 id;
        }
        // TODO: Define the rest of the registers here based on the Hardware Register Map.
    }
    pub(super) const END: usize = 0x80;
}

struct EduDriver;

#[pin_data(PinnedDrop)]
struct EduPciData<'bound> {
    pdev: ARef<pci::Device>,
    _reg: drm::Registration<'bound, EduDriver>,
}

#[pin_data]
struct EduDrmData<'drm> {
    #[pin]
    _irq: irq::Registration<'drm, EduIrqHandler<'drm>>,
}

#[pin_data]
struct EduIrqHandler<'bound> {
    pdev: &'bound pci::Device<Bound>,
    bar: pci::Bar<'bound, { regs::END }>,
}

struct EduFile;

#[pin_data]
struct EduObject {}

impl<'bound> irq::Handler for EduIrqHandler<'bound> {
    fn handle(&self) -> irq::IrqReturn {
        // TODO: Read regs::IRQ_STATUS. If it is 0, return IrqReturn::None.
        // TODO: Log the interrupt using dev_info!.
        // TODO: Write status back to IRQ_ACKNOWLEDGE to clear/acknowledge the interrupt.
        irq::IrqReturn::Handled
    }
}

impl drm::file::DriverFile for EduFile {
    type Driver = EduDriver;

    fn open(_dev: &drm::Device<EduDriver>) -> Result<Pin<KBox<Self>>> {
        Ok(KBox::new(Self, GFP_KERNEL)?.into())
    }
}

impl EduFile {
    pub(crate) fn get_id(
        _dev: &drm::Device<EduDriver, Registered>,
        reg_data: &EduDrmData<'_>,
        arg: &mut uapi::drm_edu_get_id,
        _file: &drm::File<Self>,
    ) -> Result<u32> {
        // TODO: Get the bar from reg_data._irq.handler().bar.
        // TODO: Read regs::ID and write it to arg.id.
        Ok(0)
    }

    pub(crate) fn test_liveness(
        _dev: &drm::Device<EduDriver, Registered>,
        reg_data: &EduDrmData<'_>,
        arg: &mut uapi::drm_edu_test_liveness,
        _file: &drm::File<Self>,
    ) -> Result<u32> {
        // TODO: Get the bar.
        // TODO: Write arg.val to regs::LIVENESS.
        // TODO: Read regs::LIVENESS and write it to arg.inv.
        Ok(0)
    }

    pub(crate) fn compute_factorial(
        _dev: &drm::Device<EduDriver, Registered>,
        reg_data: &EduDrmData<'_>,
        arg: &mut uapi::drm_edu_compute_factorial,
        _file: &drm::File<Self>,
    ) -> Result<u32> {
        // TODO: Get the bar.
        // TODO: Write arg.val to regs::FACTORIAL.
        // TODO: Poll STATUS.computing until it is 0 using read_poll_timeout.
        // TODO: Read result from regs::FACTORIAL and write it to arg.res.
        Ok(0)
    }

    pub(crate) fn test_irq(
        _dev: &drm::Device<EduDriver, Registered>,
        reg_data: &EduDrmData<'_>,
        arg: &mut uapi::drm_edu_test_irq,
        _file: &drm::File<Self>,
    ) -> Result<u32> {
        // TODO: Get the bar.
        // TODO: Write arg.val to regs::IRQ_RAISE to trigger interrupt.
        Ok(0)
    }
}

impl drm::gem::DriverObject for EduObject {
    type Driver = EduDriver;
    type Args = ();

    fn new(
        _dev: &drm::Device<EduDriver>,
        _size: usize,
        _args: Self::Args,
    ) -> impl PinInit<Self, Error> {
        try_pin_init!(EduObject {})
    }
}

#[vtable]
impl drm::Driver for EduDriver {
    type Data = ();
    type RegistrationData<'drm> = EduDrmData<'drm>;
    type File = EduFile;
    type Object = drm::gem::Object<EduObject>;
    type ParentDevice<Ctx: DeviceContext> = pci::Device<Ctx>;

    const INFO: drm::DriverInfo = drm::DriverInfo {
        major: 0,
        minor: 0,
        patchlevel: 0,
        name: c"qemu-edu-drm",
        desc: c"QEMU PCI EDU DRM Driver",
    };

    const FEAT_RENDER: bool = true;

    kernel::declare_drm_ioctls! {
        (EDU_GET_ID, drm_edu_get_id, ioctl::RENDER_ALLOW, EduFile::get_id),
        (EDU_TEST_LIVENESS, drm_edu_test_liveness, ioctl::RENDER_ALLOW, EduFile::test_liveness),
        (EDU_COMPUTE_FACTORIAL, drm_edu_compute_factorial, ioctl::RENDER_ALLOW, EduFile::compute_factorial),
        (EDU_TEST_IRQ, drm_edu_test_irq, ioctl::RENDER_ALLOW, EduFile::test_irq),
    }
}

impl pci::Driver for EduDriver {
    type IdInfo = ();
    type Data<'bound> = EduPciData<'bound>;

    const ID_TABLE: pci::IdTable<Self::IdInfo> = &PCI_TABLE;

    fn probe<'bound>(
        probe_pdev: &'bound pci::Device<Core<'_>>,
        _info: Option<&'bound Self::IdInfo>,
    ) -> impl PinInit<Self::Data<'bound>, Error> + 'bound {
        dev_info!(
            probe_pdev,
            "Probe QEMU EDU PCI DRM driver sample (PCI ID: {}, 0x{:x}).\n",
            probe_pdev.vendor_id(),
            probe_pdev.device_id()
        );

        // TODO: Enable PCI device memory space using enable_device_mem().
        // TODO: Set PCI device master using set_master().

        // TODO: Map BAR 0 (size 0x80) using iomap_region_sized.

        // TODO: Create UnregisteredDevice.

        // TODO: Allocate 1 IRQ vector and get the vector.

        // TODO: Request IRQ using request_irq (marked unsafe, needs safety comment!).
        // Pass EduIrqHandler initialized with probe_pdev and bar.

        // TODO: Create EduDrmData reg_data containing _irq <- irq_init.

        // TODO: Create drm::Registration.

        // TODO: Return EduPciData containing _reg.
        // Hint: You will need to use `pin_init::pin_init_scope` to initialize the driver data.
        Err(ENODEV)
    }
}

#[pinned_drop]
impl PinnedDrop for EduPciData<'_> {
    fn drop(self: Pin<&mut Self>) {
        dev_info!(self.pdev, "Remove QEMU EDU PCI DRM driver sample.\n");
    }
}

kernel::pci_device_table!(
    PCI_TABLE,
    <EduDriver as pci::Driver>::IdInfo,
    [(pci::DeviceId::from_id(pci::Vendor::QEMU, 0x11e8), ())]
);

kernel::module_pci_driver! {
    type: EduDriver,
    name: "rust_driver_pci_edu_drm",
    authors: ["Alice Ryhl"],
    description: "QEMU PCI EDU driver with DRM class device interface",
    license: "GPL v2",
}

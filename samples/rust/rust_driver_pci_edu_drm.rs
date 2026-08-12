// SPDX-License-Identifier: GPL-2.0
//! Rust PCI EDU driver sample with a DRM class device interface and IRQ support.

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
        pub(super) LIVENESS(u32) @ 0x04 {
            31:0 val;
        }
        pub(super) FACTORIAL(u32) @ 0x08 {
            31:0 val;
        }
        pub(super) STATUS(u32) @ 0x20 {
            0:0 computing;
            7:7 irq;
        }
        pub(super) IRQ_STATUS(u32) @ 0x24 {
            31:0 val;
        }
        pub(super) IRQ_RAISE(u32) @ 0x60 {
            31:0 val;
        }
    }
    pub(super) const END: usize = 0x80;
}

#[pin_data]
struct EduRegistrationData<'a> {
    pdev: &'a pci::Device<Bound>,
    #[pin]
    _irq: irq::Registration<'a, EduIrqHandler<'a>>,
}

#[pin_data]
struct EduIrqHandler<'bound> {
    pdev: &'bound pci::Device<Bound>,
    bar: pci::Bar<'bound, { regs::END }>,
}

impl<'bound> irq::Handler for EduIrqHandler<'bound> {
    fn handle(&self) -> irq::IrqReturn {
        // TODO: Read regs::IRQ_STATUS. If it is 0, return IrqReturn::None.
        // TODO: Log the interrupt using dev_info!.
        // TODO: Write status back to IRQ_STATUS to clear/acknowledge the interrupt.
        irq::IrqReturn::Handled
    }
}

struct EduFile;

impl drm::file::DriverFile for EduFile {
    type Driver = EduDrmDriver;

    fn open(_dev: &drm::Device<EduDrmDriver>) -> Result<Pin<KBox<Self>>> {
        Ok(KBox::new(Self, GFP_KERNEL)?.into())
    }
}

impl EduFile {
    pub(crate) fn get_id(
        _dev: &drm::Device<EduDrmDriver, Registered>,
        reg_data: &EduRegistrationData<'_>,
        arg: &mut uapi::drm_edu_get_id,
        _file: &drm::File<Self>,
    ) -> Result<u32> {
        // TODO: Get the bar from reg_data._irq.handler().bar.
        // TODO: Read regs::ID and write it to arg.id.
        Ok(0)
    }

    pub(crate) fn test_liveness(
        _dev: &drm::Device<EduDrmDriver, Registered>,
        reg_data: &EduRegistrationData<'_>,
        arg: &mut uapi::drm_edu_test_liveness,
        _file: &drm::File<Self>,
    ) -> Result<u32> {
        // TODO: Get the bar.
        // TODO: Write arg.val to regs::LIVENESS.
        // TODO: Read regs::LIVENESS and write it to arg.inv.
        Ok(0)
    }

    pub(crate) fn compute_factorial(
        _dev: &drm::Device<EduDrmDriver, Registered>,
        reg_data: &EduRegistrationData<'_>,
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
        _dev: &drm::Device<EduDrmDriver, Registered>,
        reg_data: &EduRegistrationData<'_>,
        arg: &mut uapi::drm_edu_test_irq,
        _file: &drm::File<Self>,
    ) -> Result<u32> {
        // TODO: Get the bar.
        // TODO: Write arg.val to regs::IRQ_RAISE to trigger interrupt.
        Ok(0)
    }
}

#[pin_data]
struct EduObject {}

impl drm::gem::DriverObject for EduObject {
    type Driver = EduDrmDriver;
    type Args = ();

    fn new(
        _dev: &drm::Device<EduDrmDriver>,
        _size: usize,
        _args: Self::Args,
    ) -> impl PinInit<Self, Error> {
        try_pin_init!(EduObject {})
    }
}

struct EduDrmDriver;

#[vtable]
impl drm::Driver for EduDrmDriver {
    type Data = ();
    type RegistrationData<'drm> = EduRegistrationData<'drm>;
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

#[pin_data(PinnedDrop)]
struct EduDriverData<'bound> {
    pdev: ARef<pci::Device>,
    _reg: drm::Registration<'bound, EduDrmDriver>,
}

struct EduDriver;

kernel::pci_device_table!(
    PCI_TABLE,
    <EduDriver as pci::Driver>::IdInfo,
    [(pci::DeviceId::from_id(pci::Vendor::QEMU, 0x11e8), ())]
);

impl pci::Driver for EduDriver {
    type IdInfo = ();
    type Data<'bound> = EduDriverData<'bound>;

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

        // TODO: Create EduRegistrationData reg_data containing _irq <- irq_init.

        // TODO: Create drm::Registration.

        // TODO: Return EduDriverData containing _reg.
        // Hint: You will need to use `pin_init::pin_init_scope` to initialize the driver data.
        Err(ENODEV)
    }
}

#[pinned_drop]
impl PinnedDrop for EduDriverData<'_> {
    fn drop(self: Pin<&mut Self>) {
        dev_info!(self.pdev, "Remove QEMU EDU PCI DRM driver sample.\n");
    }
}

kernel::module_pci_driver! {
    type: EduDriver,
    name: "rust_driver_pci_edu_drm",
    authors: ["Alice Ryhl"],
    description: "QEMU PCI EDU driver with DRM class device interface",
    license: "GPL v2",
}

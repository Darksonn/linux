// SPDX-License-Identifier: GPL-2.0
#![allow(unused_variables, unused_imports, dead_code)]
//! Rust PCI EDU driver exercise with a DRM class device interface.

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
        // TODO: Define the hardware registers here.
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
    bar: pci::Bar<'drm, { regs::END }>,
}

struct EduFile;

#[pin_data]
struct EduObject {}

impl drm::file::DriverFile for EduFile {
    type Driver = EduDriver;

    fn open(_dev: &drm::Device<EduDriver>) -> Result<Pin<KBox<Self>>> {
        Ok(KBox::new(Self, GFP_KERNEL)?.into())
    }
}

impl EduFile {
    // TODO: Implement IOCTL callbacks here.
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
        major: 1,
        minor: 0,
        patchlevel: 0,
        name: c"qemu-edu-drm",
        desc: c"QEMU PCI EDU DRM Driver",
    };

    const FEAT_RENDER: bool = true;

    kernel::declare_drm_ioctls! {
        // TODO: Declare DRM IOCTLs here.
    }
}

impl pci::Driver for EduDriver {
    type IdInfo = ();
    type Data<'bound> = EduPciData<'bound>;

    const ID_TABLE: pci::IdTable<Self::IdInfo> = &PCI_TABLE;

    fn probe<'bound>(
        pdev: &'bound pci::Device<Core<'_>>,
        _info: Option<&'bound Self::IdInfo>,
    ) -> impl PinInit<Self::Data<'bound>, Error> + 'bound {
        pin_init::pin_init_scope(move || {
            dev_info!(
                pdev,
                "Probe QEMU EDU PCI DRM driver (PCI ID: {}, 0x{:x}).\n",
                pdev.vendor_id(),
                pdev.device_id()
            );

            pdev.enable_device_mem()?;
            pdev.set_master();

            let bar = pdev.iomap_region_sized::<{ regs::END }>(0, c"qemu_edu_drm")?;
            let unreg_dev = drm::UnregisteredDevice::<EduDriver>::new(pdev, Ok(()))?;
            let reg_data = try_pin_init!(EduDrmData { bar });

            // SAFETY: `_reg` is stored in `EduPciData` and dropped when the PCI driver is
            // unbound; it is never forgotten.
            let _reg = unsafe {
                drm::Registration::new(pdev.as_ref(), unreg_dev, reg_data, 0)?
            };

            Ok(try_pin_init!(EduPciData {
                pdev: pdev.into(),
                _reg,
            }))
        })
    }
}

#[pinned_drop]
impl PinnedDrop for EduPciData<'_> {
    fn drop(self: Pin<&mut Self>) {
        dev_info!(self.pdev, "Remove QEMU EDU PCI DRM driver.\n");
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
    authors: ["Your Name"],
    description: "QEMU PCI EDU driver with DRM class device interface",
    license: "GPL v2",
}

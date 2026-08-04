// SPDX-License-Identifier: GPL-2.0

//! Rust PCI EDU driver sample with a miscdevice interface and IRQ support.
//!
//! To make this driver probe, QEMU must be run with `-device edu`.
//!
//! This sample demonstrates how to combine the `pci::Driver`, `miscdevice`, and `irq::Handler`
//! abstractions to create a driver that operates a PCI device via a miscdevice interface (`/dev/qemu-edu`).
//!
//! # Example userspace C program
//!
//! Below is an example C program that exercises `/dev/qemu-edu` via IOCTLs:
//!
//! ```c
//! #include <stdio.h>
//! #include <stdlib.h>
//! #include <errno.h>
//! #include <fcntl.h>
//! #include <unistd.h>
//! #include <sys/ioctl.h>
//! #include <stdint.h>
//!
//! #define RUST_EDU_GET_ID            _IOR('E', 0x00, uint32_t)
//! #define RUST_EDU_TEST_LIVENESS     _IOWR('E', 0x01, uint32_t)
//! #define RUST_EDU_COMPUTE_FACTORIAL _IOWR('E', 0x02, uint32_t)
//! #define RUST_EDU_TEST_IRQ          _IOW('E', 0x03, uint32_t)
//!
//! int main() {
//!   int fd, ret;
//!   uint32_t val;
//!
//!   printf("Opening /dev/qemu-edu\n");
//!   fd = open("/dev/qemu-edu", O_RDWR);
//!   if (fd < 0) {
//!     perror("open");
//!     return errno;
//!   }
//!
//!   printf("Fetching EDU identification register\n");
//!   ret = ioctl(fd, RUST_EDU_GET_ID, &val);
//!   if (ret < 0) {
//!     perror("ioctl: RUST_EDU_GET_ID failed");
//!     close(fd);
//!     return errno;
//!   }
//!   printf("EDU ID: 0x%08x\n", val);
//!
//!   val = 0x12345678;
//!   printf("Testing card liveness with value 0x%08x\n", val);
//!   ret = ioctl(fd, RUST_EDU_TEST_LIVENESS, &val);
//!   if (ret < 0) {
//!     perror("ioctl: RUST_EDU_TEST_LIVENESS failed");
//!     close(fd);
//!     return errno;
//!   }
//!   printf("Liveness returned: 0x%08x (expected 0x%08x)\n", val, ~0x12345678);
//!
//!   val = 5;
//!   printf("Computing factorial of %u\n", val);
//!   ret = ioctl(fd, RUST_EDU_COMPUTE_FACTORIAL, &val);
//!   if (ret < 0) {
//!     perror("ioctl: RUST_EDU_COMPUTE_FACTORIAL failed");
//!     close(fd);
//!     return errno;
//!   }
//!   printf("Factorial of 5 is %u (expected 120)\n", val);
//!
//!   val = 0x100;
//!   printf("Testing IRQ raise with value 0x%08x\n", val);
//!   ret = ioctl(fd, RUST_EDU_TEST_IRQ, &val);
//!   if (ret < 0) {
//!     perror("ioctl: RUST_EDU_TEST_IRQ failed");
//!     close(fd);
//!     return errno;
//!   }
//!   printf("IRQ test triggered successfully\n");
//!
//!   close(fd);
//!   printf("Success\n");
//!   return 0;
//! }
//! ```

use kernel::{
    device,
    device::{Bound, Core},
    devres::Devres,
    fs::File,
    io::{register, Io},
    ioctl::{_IOC_SIZE, _IOR, _IOW, _IOWR},
    irq,
    miscdevice::{MiscDevice, MiscDeviceOptions, MiscDeviceRegistration},
    pci,
    prelude::*,
    sync::aref::ARef,
    sync::Arc,
    types::ForeignOwnable,
    uaccess::{UserPtr, UserSlice},
};

const EDU_GET_ID: u32 = _IOR::<u32>('E' as u32, 0x00);
const EDU_TEST_LIVENESS: u32 = _IOWR::<u32>('E' as u32, 0x01);
const EDU_COMPUTE_FACTORIAL: u32 = _IOWR::<u32>('E' as u32, 0x02);
const EDU_TEST_IRQ: u32 = _IOW::<u32>('E' as u32, 0x03);

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

struct EduDevice {
    pdev: ARef<pci::Device>,
    bar: Devres<pci::Bar<'static, { regs::END }>>,
}

#[pin_data]
struct EduIrqHandler {
    edu: Arc<EduDevice>,
}

impl irq::Handler for EduIrqHandler {
    fn handle(&self, _device: &device::Device<Bound>) -> irq::IrqReturn {
        let bar = match self.edu.bar.try_access() {
            Some(bar) => bar,
            None => return irq::IrqReturn::None,
        };

        let status = bar.read(regs::IRQ_STATUS).val();
        if status == 0 {
            return irq::IrqReturn::None;
        }

        dev_info!(self.edu.pdev, "QEMU EDU IRQ handled! status=0x{:x}\n", status);

        bar.write_reg(regs::IRQ_STATUS::zeroed().with_val(status));

        irq::IrqReturn::Handled
    }
}

#[pin_data]
struct EduMiscDevice {
    edu: Arc<EduDevice>,
}

#[vtable]
impl MiscDevice for EduMiscDevice {
    type Data = Arc<EduDevice>;
    type Ptr = Pin<KBox<Self>>;

    fn open(_file: &File, misc: &MiscDeviceRegistration<Self>) -> Result<Pin<KBox<Self>>> {
        let edu = misc.data().clone();

        dev_info!(edu.pdev, "Opening QEMU EDU PCI Misc Device\n");

        KBox::try_pin_init(
            try_pin_init! {
                EduMiscDevice { edu }
            },
            GFP_KERNEL,
        )
    }

    fn ioctl(
        me: <Self::Ptr as ForeignOwnable>::Borrowed<'_>,
        _file: &File,
        cmd: u32,
        arg: usize,
    ) -> Result<isize> {
        let arg = UserPtr::from_addr(arg);
        let size = _IOC_SIZE(cmd);
        let bar = me.edu.bar.try_access().ok_or(ENODEV)?;

        match cmd {
            EDU_GET_ID => {
                let id = bar.read(regs::ID).id();
                UserSlice::new(arg, size).writer().write::<u32>(&id)?;
            }
            EDU_TEST_LIVENESS => {
                let mut reader = UserSlice::new(arg, size).reader();
                let val = reader.read::<u32>()?;
                bar.write_reg(regs::LIVENESS::zeroed().with_val(val));
                let inv = bar.read(regs::LIVENESS).val();
                UserSlice::new(arg, size).writer().write::<u32>(&inv)?;
            }
            EDU_COMPUTE_FACTORIAL => {
                let mut reader = UserSlice::new(arg, size).reader();
                let val = reader.read::<u32>()?;
                bar.write_reg(regs::FACTORIAL::zeroed().with_val(val));

                while bar.read(regs::STATUS).computing() != 0 {
                    core::hint::spin_loop();
                }

                let res = bar.read(regs::FACTORIAL).val();
                UserSlice::new(arg, size).writer().write::<u32>(&res)?;
            }
            EDU_TEST_IRQ => {
                let mut reader = UserSlice::new(arg, size).reader();
                let val = reader.read::<u32>()?;
                bar.write_reg(regs::IRQ_RAISE::zeroed().with_val(val));
            }
            _ => return Err(ENOTTY),
        }

        Ok(0)
    }
}

#[pin_data(PinnedDrop)]
struct EduDriverData {
    pdev: ARef<pci::Device>,
    edu: Arc<EduDevice>,
    #[pin]
    _irq: irq::Registration<EduIrqHandler>,
    #[pin]
    _miscdev: MiscDeviceRegistration<EduMiscDevice>,
}

struct EduDriver;

kernel::pci_device_table!(
    PCI_TABLE,
    MODULE_PCI_TABLE,
    <EduDriver as pci::Driver>::IdInfo,
    [(
        pci::DeviceId::from_id(pci::Vendor::QEMU, 0x11e8),
        ()
    )]
);

impl pci::Driver for EduDriver {
    type IdInfo = ();
    type Data<'bound> = EduDriverData;

    const ID_TABLE: pci::IdTable<Self::IdInfo> = &PCI_TABLE;

    fn probe<'bound>(
        pdev: &'bound pci::Device<Core<'_>>,
        _info: &'bound Self::IdInfo,
    ) -> impl PinInit<Self::Data<'bound>, Error> + 'bound {
        pin_init::pin_init_scope(move || {
            dev_info!(
                pdev,
                "Probe QEMU EDU PCI driver sample (PCI ID: {}, 0x{:x}).\n",
                pdev.vendor_id(),
                pdev.device_id()
            );

            pdev.enable_device_mem()?;
            pdev.set_master();

            let bar = pdev
                .iomap_region_sized::<{ regs::END }>(0, c"qemu_edu")?
                .into_devres()?;

            let edu = Arc::new(
                EduDevice {
                    pdev: pdev.into(),
                    bar,
                },
                GFP_KERNEL,
            )?;

            let vectors =
                pdev.alloc_irq_vectors(1, 1, pci::IrqTypes::all())?;
            let vector = *vectors.start();

            let edu_irq = edu.clone();
            let irq_init = pdev.request_irq(
                vector,
                irq::Flags::SHARED,
                c"qemu_edu",
                try_pin_init!(EduIrqHandler {
                    edu: edu_irq,
                }),
            );

            let options = MiscDeviceOptions {
                name: c"qemu-edu",
                parent: Some(pdev.as_ref()),
            };

            Ok(try_pin_init!(EduDriverData {
                pdev: pdev.into(),
                edu: edu.clone(),
                _irq <- irq_init,
                _miscdev <- MiscDeviceRegistration::register(options, edu.clone()),
            }))
        })
    }
}

#[pinned_drop]
impl PinnedDrop for EduDriverData {
    fn drop(self: Pin<&mut Self>) {
        dev_info!(self.pdev, "Remove QEMU EDU PCI driver sample.\n");
    }
}

kernel::module_pci_driver! {
    type: EduDriver,
    name: "rust_driver_pci_edu",
    authors: ["Alice Ryhl"],
    description: "QEMU PCI EDU driver with miscdevice interface",
    license: "GPL v2",
}

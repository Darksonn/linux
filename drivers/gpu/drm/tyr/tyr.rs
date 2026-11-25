// SPDX-License-Identifier: GPL-2.0 or MIT

//! Arm Mali Tyr DRM driver.
//!
//! The name "Tyr" is inspired by Norse mythology, reflecting Arm's tradition of
//! naming their GPUs after Nordic mythological figures and places.

// `as` conversions can silently truncate, so they are denied by default in
// this driver. A conversion that cannot be avoided is justified in a
// `// CAST:` comment and marked with `#[expect(clippy::as_conversions)]`.
#![deny(clippy::as_conversions)]

use crate::driver::TyrPlatformDriver;

mod driver;
mod file;
mod fw;
mod gem;
mod gpu;
mod mmu;
mod regs;
mod slot;
mod vm;

kernel::module_platform_driver! {
    type: TyrPlatformDriver,
    name: "tyr",
    authors: ["The Tyr driver authors"],
    description: "Arm Mali Tyr DRM driver",
    license: "Dual MIT/GPL",
}

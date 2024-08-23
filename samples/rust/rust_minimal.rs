// SPDX-License-Identifier: GPL-2.0

//! Rust minimal sample.

use kernel::prelude::*;
use kernel::sync::Arc;

module! {
    type: RustMinimal,
    name: "rust_minimal",
    author: "Rust for Linux Contributors",
    description: "Rust minimal sample",
    license: "GPL",
}

struct RustMinimal {
    numbers: Vec<i32>,
}

trait MyTrait {
    fn my_fn(self: Arc<Self>);
}
impl MyTrait for Vec<i32> {
    fn my_fn(self: Arc<Self>) {
        pr_info!("{:?}", self.as_slice());
    }
}

impl kernel::Module for RustMinimal {
    fn init(_module: &'static ThisModule) -> Result<Self> {
        pr_info!("Rust minimal sample (init)\n");
        pr_info!("Am I built-in? {}\n", !cfg!(MODULE));

        let mut numbers = Vec::new();
        numbers.push(72, GFP_KERNEL)?;
        numbers.push(108, GFP_KERNEL)?;
        numbers.push(200, GFP_KERNEL)?;

        let in_arc = kernel::sync::Arc::new(numbers)?;
        in_arc.my_fn();
        let arc_dyn: Arc<dyn MyTrait> = in_arc;
        arc_dyn.my_fn();

        Ok(RustMinimal { numbers })
    }
}

impl Drop for RustMinimal {
    fn drop(&mut self) {
        pr_info!("My numbers are {:?}\n", self.numbers);
        pr_info!("Rust minimal sample (exit)\n");
    }
}

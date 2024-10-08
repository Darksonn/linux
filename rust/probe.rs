//! Nearly empty file passed to rustc-option by Make.
//!
//! The no_core attribute is needed because rustc-option otherwise fails due to
//! not being able to find the core part of the standard library.

#![feature(no_core)]
#![no_core]

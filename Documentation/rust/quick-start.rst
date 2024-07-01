.. SPDX-License-Identifier: GPL-2.0

Quick Start
===========

This document describes how to get started with kernel development in Rust.

There are a few ways to install a Rust toolchain needed for kernel development.
A simple way is to use the packages from your Linux distribution if they are
suitable -- the first section below explains this approach. An advantage of this
approach is that, typically, the distribution will match the LLVM used by Rust
and Clang.

Alternatively, the next two "Requirements" sections explain each component and
how to install them through ``rustup``, the standalone installers from Rust
and/or building them.

The rest of the document explains other aspects on how to get started.


Distributions
-------------

Arch Linux
**********

Arch Linux provides recent Rust releases and thus it should generally work out
of the box, e.g.::

	pacman -S rust rust-src rust-bindgen


Debian
******

Debian Unstable (Sid), outside of the freeze period, provides recent Rust
release and thus it should generally work out of the box, e.g.::

	apt install rustc rust-src bindgen rustfmt rust-clippy


Fedora Linux
************

Fedora Linux provides recent Rust releases and thus it should generally work out
of the box, e.g.::

	dnf install rust rust-src bindgen-cli rustfmt clippy


Gentoo Linux
************

Gentoo Linux (and especially the testing branch) provides recent Rust releases
and thus it should generally work out of the box, e.g.::

	USE='rust-src rustfmt clippy' emerge dev-lang/rust dev-util/bindgen

``LIBCLANG_PATH`` may need to be set.


Nix
***

Nix (unstable channel) provides recent Rust releases and thus it should
generally work out of the box, e.g.::

	{ pkgs ? import <nixpkgs> {} }:
	pkgs.mkShell {
	  nativeBuildInputs = with pkgs; [ rustc rust-bindgen rustfmt clippy ];
	  RUST_LIB_SRC = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";
	}


openSUSE
********

openSUSE Tumbleweed provides recent Rust releases and thus it should generally
work out of the box, e.g.::

	zypper install rust rust-bindgen clang

The Rust standard library source code, ``rustfmt`` and Clippy are not packaged
(yet), thus currently they need to be installed separately.


Requirements: Building
----------------------

This section explains how to fetch the tools needed for building.

To easily check whether the requirements are met, the following target
can be used::

	make LLVM=1 rustavailable

This triggers the same logic used by Kconfig to determine whether
``RUST_IS_AVAILABLE`` should be enabled; but it also explains why not
if that is the case.


rustc
*****

A recent version of the Rust compiler is required.

If ``rustup`` is being used, enter the kernel build directory (or use
``--path=<build-dir>`` argument to the ``set`` sub-command) and run,
for instance::

	rustup override set $(scripts/min-tool-version.sh rustc)

This will configure your working directory to use the given version of
``rustc`` without affecting your default toolchain.

Note that the override applies to the current working directory (and its
sub-directories).

If you are not using ``rustup``, fetch a standalone installer from:

	https://forge.rust-lang.org/infra/other-installation-methods.html#standalone


Rust standard library source
****************************

The Rust standard library source is required because the build system will
cross-compile ``core`` and ``alloc``.

If ``rustup`` is being used, run::

	rustup component add rust-src

The components are installed per toolchain, thus upgrading the Rust compiler
version later on requires re-adding the component.

Otherwise, if a standalone installer is used, the Rust source tree may be
downloaded into the toolchain's installation folder::

	curl -L "https://static.rust-lang.org/dist/rust-src-$(scripts/min-tool-version.sh rustc).tar.gz" |
		tar -xzf - -C "$(rustc --print sysroot)/lib" \
		"rust-src-$(scripts/min-tool-version.sh rustc)/rust-src/lib/" \
		--strip-components=3

In this case, upgrading the Rust compiler version later on requires manually
updating the source tree (this can be done by removing ``$(rustc --print
sysroot)/lib/rustlib/src/rust`` then rerunning the above command).


libclang
********

``libclang`` (part of LLVM) is used by ``bindgen`` to understand the C code
in the kernel, which means LLVM needs to be installed; like when the kernel
is compiled with ``LLVM=1``.

Linux distributions are likely to have a suitable one available, so it is
best to check that first.

There are also some binaries for several systems and architectures uploaded at:

	https://releases.llvm.org/download.html

Otherwise, building LLVM takes quite a while, but it is not a complex process:

	https://llvm.org/docs/GettingStarted.html#getting-the-source-code-and-building-llvm

Please see Documentation/kbuild/llvm.rst for more information and further ways
to fetch pre-built releases and distribution packages.


bindgen
*******

The bindings to the C side of the kernel are generated at build time using
the ``bindgen`` tool.

Install it, for instance, via (note that this will download and build the tool
from source)::

	cargo install --locked --version $(scripts/min-tool-version.sh bindgen) bindgen-cli

``bindgen`` uses the ``clang-sys`` crate to find a suitable ``libclang`` (which
may be linked statically, dynamically or loaded at runtime). By default, the
``cargo`` command above will produce a ``bindgen`` binary that will load
``libclang`` at runtime. If it is not found (or a different ``libclang`` than
the one found should be used), the process can be tweaked, e.g. by using the
``LIBCLANG_PATH`` environment variable. For details, please see ``clang-sys``'s
documentation at:

	https://github.com/KyleMayes/clang-sys#linking

	https://github.com/KyleMayes/clang-sys#environment-variables


Requirements: Developing
------------------------

This section explains how to fetch the tools needed for developing. That is,
they are not needed when just building the kernel.


rustfmt
*******

The ``rustfmt`` tool is used to automatically format all the Rust kernel code,
including the generated C bindings (for details, please see
coding-guidelines.rst).

If ``rustup`` is being used, its ``default`` profile already installs the tool,
thus nothing needs to be done. If another profile is being used, the component
can be installed manually::

	rustup component add rustfmt

The standalone installers also come with ``rustfmt``.


clippy
******

``clippy`` is a Rust linter. Running it provides extra warnings for Rust code.
It can be run by passing ``CLIPPY=1`` to ``make`` (for details, please see
general-information.rst).

If ``rustup`` is being used, its ``default`` profile already installs the tool,
thus nothing needs to be done. If another profile is being used, the component
can be installed manually::

	rustup component add clippy

The standalone installers also come with ``clippy``.


cargo
*****

``cargo`` is the Rust native build system. It is currently required to run
the tests since it is used to build a custom standard library that contains
the facilities provided by the custom ``alloc`` in the kernel. The tests can
be run using the ``rusttest`` Make target.

If ``rustup`` is being used, all the profiles already install the tool,
thus nothing needs to be done.

The standalone installers also come with ``cargo``.


rustdoc
*******

``rustdoc`` is the documentation tool for Rust. It generates pretty HTML
documentation for Rust code (for details, please see
general-information.rst).

``rustdoc`` is also used to test the examples provided in documented Rust code
(called doctests or documentation tests). The ``rusttest`` Make target uses
this feature.

If ``rustup`` is being used, all the profiles already install the tool,
thus nothing needs to be done.

The standalone installers also come with ``rustdoc``.


rust-analyzer
*************

The `rust-analyzer <https://rust-analyzer.github.io/>`_ language server can
be used with many editors to enable syntax highlighting, completion, go to
definition, and other features.

``rust-analyzer`` needs a configuration file, ``rust-project.json``, which
can be generated by the ``rust-analyzer`` Make target::

	make LLVM=1 rust-analyzer


Configuration
-------------

``Rust support`` (``CONFIG_RUST``) needs to be enabled in the ``General setup``
menu. The option is only shown if a suitable Rust toolchain is found (see
above), as long as the other requirements are met. In turn, this will make
visible the rest of options that depend on Rust.

Afterwards, go to::

	Kernel hacking
	    -> Sample kernel code
	        -> Rust samples

And enable some sample modules either as built-in or as loadable.


Building
--------

Building a kernel with a complete LLVM toolchain is the best supported setup
at the moment. That is::

	make LLVM=1

Using GCC also works for some configurations, but it is very experimental at
the moment.


Hacking
-------

To dive deeper, take a look at the source code of the samples
at ``samples/rust/``, the Rust support code under ``rust/`` and
the ``Rust hacking`` menu under ``Kernel hacking``.

If GDB/Binutils is used and Rust symbols are not getting demangled, the reason
is the toolchain does not support Rust's new v0 mangling scheme yet.
There are a few ways out:

  - Install a newer release (GDB >= 10.2, Binutils >= 2.36).

  - Some versions of GDB (e.g. vanilla GDB 10.1) are able to use
    the pre-demangled names embedded in the debug info (``CONFIG_DEBUG_INFO``).

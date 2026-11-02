// SPDX-License-Identifier: GPL-3.0-or-later

//! # General
#![doc = include_str!("../docs/REPO.md")]

/*! # Library
 *
 * kickit is an immutable init system written in Rust, with support for service sandboxing. This library
 * provides various definitions & macros used by the init system & its helper frontend binary [ktctl].
 *
 * Additionally, it is provided as a crate, if anyone wishes to write binaries to interact with the init
 * system.
 *
 * **See [the GitHub page] for more information.**
 *
 * ## Sockets
 *
 * Sockets are utilized as the main I/O form between `ktctl` and the init system. They are implemented
 * using the [Tokio] crate (also used for service implementation & more!).
 *
 * However, I/O with these sockets is not limited to just `ktctl`. If you need to use these sockets in
 * your Rust application, refer to the [socket] module for ABI definitions, and [`ktctl::socket`] for I/O.
 *
 *
 * [init]: mod@init
 * [ktctl]: mod@ktctl
 *
 * [socket]: mod@socket
 *
 * [the GitHub page]: https://github.com/h4dynn/kickit
 * [Tokio]: https://tokio.rs/
 */

#![allow(unused_parens)]
#![allow(non_snake_case)]

extern crate ruzstd;
extern crate thiserror;
extern crate derive_more;

#[macro_use]
mod macros;

// Internal modules - see their respective files for more information
modules! {
  // Public visibility for `kickit` and `warden` binary
  #[internal(pub)]
  mod niceties;

  #[internal(pub)]
  mod helper_types;
}

pub mod init;
pub mod ktctl;
pub mod console;
pub mod state;
pub mod socket;

/// Ambiguous owned instance of raw binary data
pub type Data = Vec<u8>;
/// Double-ended sister alias of [`Data`], optimized for front-end manipulation
pub type DequeData = std::collections::VecDeque<u8>;

/// Identical to [`String`] except for having a fixed size (making this type 1 byte smaller, wow)
pub type BoxedStr = Box<str>;
/// Annotated return type for functions that block indefinitely
pub type AsyncBlock = std::convert::Infallible;

/// Version of cargo package in string form
#[cfg(feature = "stable")]
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Version of cargo package suffixed with unstable
#[cfg(not(feature = "stable"))]
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (unstable)");

/// Library directory containing target & service configurations
pub const PREFIX: &str = "/usr/lib/kickit";

// Detect this at compile-time to avoid headaches
#[cfg(not(target_os = "linux"))]
compile_error!("unsupported platform; only Linux is supported");

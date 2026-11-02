/*!
 * Frontend access of the init system
 *
 * # Mechanism
 *
 * This application does not modify anything directly (e.g. killing services itself, by a signal). Instead,
 * it relies on the init system to perform requested tasks. This means for this frontend to actually do a
 * task, [the init socket module] has to verify it is okay with it. This is ideal for security purposes.
 *
 * [the init socket module]: crate::init::socket
 *
 * # Limitations
 *
 * ## Services
 *
 * Usually, init systems will allow the user to kill a service when they wish (even if its critical for the
 * system, the beauty of Linux!). However, this is forbidden by `kickit` because services are designed to
 * follow a fixed lifetime, that is: once they have been started, they run constantly until system shutdown
 * (understandably this may seem odd,
 * [but I promise you there's a reason behind it](https://github.com/h4dynn/kickit#design))
 */

pub mod console;
pub mod socket;
pub mod service;

// ASCII logo
#[doc(hidden)]
pub const LOGO: &str = include_str!("../../assets/logo.txt");

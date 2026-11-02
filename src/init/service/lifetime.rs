/*!
 * # Service lifetime control
 *
 * This module includes implementations for supervising standard services. By default, the [Root]
 * lifetime is used. For user services, a seperate [User] lifetime is used
 */

use std::future;
use crate::{AsyncBlock, init::{console::prelude::*, service::Supervisor, power::POWER_OFF_READY}};

/// Enforcement rules for service process supervision
pub trait ServiceLifetime
{
  /**
    * Supervise a daemon-like service ([Standard]) to make sure it never dies
    *
    * [Standard]: super::Pattern::Standard
    */
  fn supervise(supervisor: &mut Supervisor) -> impl Future<Output = Result<AsyncBlock>> + Send;
}

/**
  * # Root service lifetime
  *
  * Used for services executed at the init-level, as the root user. Standard services are supervised
  * to forbid sudden exits. On exit, an error will be thrown (except for power-off).
  *
  * ## Design
  *
  * ### Example standard service (supervisor task)
  *
  * Simplified code:
  *
  * ```rust
  * // Block until the process returns
  * supervisor.process.wait().await;
  *
  * // On power-off, the process is expected to return, so prevent an error being thrown
  * if (!POWER_OFF_READY)
  * {
  *   return Err(Error::ServiceDown.trace("service died during supervision").subject(supervisor.name))
  * }
  * ```
  */
#[derive(PartialEq, Eq, Copy, Clone, Debug, Default, derive_more::Display)]
pub struct Root;

/**
  * # User service lifetime
  *
  * As these services are ran under the user's profile, not root, we can be alot more leanient-
  * and allow these services to be killed at anytime by the user.
  */
#[derive(PartialEq, Eq, Copy, Clone, Debug, Default, derive_more::Display)]
pub struct User;

impl ServiceLifetime for Root
{
  /**
    * # Errors
    *
    * * Service process exited on a code (even if the code is 0)
    *
    * * Failed to wait for the process
    */
  fn supervise(supervisor: &mut Supervisor) -> impl Future<Output = Result<AsyncBlock>>
  {
    // This method will cause this function to block until process death
    let res = supervisor.process.wait();

    if (POWER_OFF_READY.get() != Some(&true))
    {
      return future::ready(Err(match (res.map(|status| status.code()))
      {
        Ok(Some(c)) => Error::ServiceDown.trace(format!("process died during supervision (code: {c})")),
        Ok(None) => Error::ServiceDown.trace("process died during supervision (code unavailable)"),
        // Extremely unlikely to happen but we still account for it just in case
        Err(error) => Error::Unknown.trace(format!("service process supervision error: {error}"))
      }
        .subject(&supervisor.name)))
    }

    loop { std::thread::park() }
  }
}

impl ServiceLifetime for User
{
  async fn supervise(_supervisor: &mut Supervisor) -> Result<AsyncBlock>
  {
    todo!()
  }
}

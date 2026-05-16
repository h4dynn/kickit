//! General implementation for logging, errors and status updates

use std::fmt::Display;
use crate::display_enum;

/*
 * Implementation for throwing an error with a trace (ErrorTrace) or
 * without a trace (Error)
 */
pub trait ReturnError
{
  // The ! return type shows that fatal() should never return anything
  fn fatal(self) -> !;
  fn warn(self);
}

pub trait HandleError: Sized
{
  type OkType;
  type ErrorType: ReturnError;
  // handle() functions like unwrap(): Return contents if OK or fatal if not
  fn handle(self) -> Self::OkType;
  // Do nothing with OK result, warn on error
  fn or_warn(self);
}

pub trait ExtendWithContext<OkType, ErrorType>
{
  /**
    * Add context to an existing trace error
    *
    * # Errors
    * - Result is of error variant
    */
  fn context(self, context: impl Display) -> Result<OkType, ErrorType>;
}

pub enum Colour { RESET, BOLD, RED, ORANGE, GREEN }

display_enum!
{
  Colour {
    RESET => "\x1b[0m", BOLD => "\x1b[0;1m", RED => "\x1b[0;1;31m",
    ORANGE => "\x1b[0;1;33m", GREEN => "\x1b[0;1;92m"
  }
}

/*
 * What to do when a Error is found
 * Implemented for Result<anything, error> and Option<error>
 */
impl<S, F: ReturnError> HandleError for Result<S, F>
{
  type OkType = S;
  type ErrorType = F;

  fn handle(self) -> Self::OkType { match (self) { Ok(c) => c, Err(e) => e.fatal() } }
  fn or_warn(self) { if let Err(e) = self { e.warn() } }
}

// Assume here that the Option carries an error
impl<F: ReturnError> HandleError for Option<F>
{
  type OkType = ();
  type ErrorType = F;

  fn handle(self) -> Self::OkType { if let Some(error) = self { error.fatal() } }
  fn or_warn(self) { if let Some(e) = self { e.warn() } }
}

// Like assert!() but less panicky
#[macro_export]
macro_rules! affirm { ($t: expr, $f: expr) => { if (!$t) { return Err($f) } }; }

/*
 * Re-export our macro, so it matches our current module path
 * (i.e. it exports to crate::console::affirm as well as crate::affirm)
 * (why doesn't rust do this automatically already???)
 */
pub use crate::affirm as affirm;

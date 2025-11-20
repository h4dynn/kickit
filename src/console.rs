//! General implementation for logging, errors and status updates

use crate::display_enum;

/*
 * Implementation for throwing an error with a trace (KTErrorTrace) or
 * without a trace (KTError)
 */
pub trait ReturnError
{
  // The ! return type shows that fatal() should never return anything
  fn fatal(self) -> !;
  fn warn(self);
}

pub trait HandleKTError: Sized
{
  type OkType;
  type ErrorType: ReturnError;

  // handle() functions like unwrap(): Return contents if OK or fatal if not
  fn handle(self) -> Self::OkType;

  // Do nothing with OK result, warn on error
  fn or_warn(self);
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
 * What to do when a KTError is found
 * Implemented for Result<anything, error> and Option<error>
 */
impl<S, F: ReturnError> HandleKTError for Result<S, F>
{
  type OkType = S;
  type ErrorType = F;

  fn handle(self) -> Self::OkType { match (self) { Ok(c) => c, Err(e) => e.fatal() } }
  fn or_warn(self) { if let Err(e) = self { e.warn() } }
}

// Assume here that the Option carries an error
impl<F: ReturnError> HandleKTError for Option<F>
{
  type OkType = ();
  type ErrorType = F;

  fn handle(self) -> Self::OkType { if let Some(error) = self { error.fatal() } }
  fn or_warn(self) { if let Some(e) = self { e.warn() } }
}



// Like assert!() but less panicky
#[macro_export]
macro_rules! affirm { ($t: expr, $f: expr) => { if (!$t) { return Err($f) } }; }

//! Init state management

use std::sync::Mutex;
use crate::display_enum;

#[derive(PartialEq, Eq, Clone, Copy, Debug, Default)]
#[must_use]
pub enum InitState { #[default] Ok = 0x02, Emergency = 0xa8, Stalled = 0xdc, Down = 0x20 }

// Setup a global mutex for the state, this can change once locked
pub static INIT_STATE: Mutex<InitState> = Mutex::new(InitState::Ok);

display_enum!
{
  InitState { Ok => "Running", Emergency => "Emergency", Stalled => "Stalled", Down => "Down" }
}

impl From<u8> for InitState
{
  fn from(byte: u8) -> Self
  {
    use InitState::{Ok, Emergency, Stalled, Down};

    /*
     * Rust matching doesn't allow us to put a `x as y` as a
     * match pattern, so create constants for this instead
     */
    const OK: u8 = Ok as u8;
    const EMERGENCY: u8 = Emergency as u8;
    const STALLED: u8 = Stalled as u8;

    match (byte)
    {
      OK => Ok,
      EMERGENCY => Emergency,
      STALLED => Stalled,
      _ => Down
    }
  }
}

impl InitState
{
  #[must_use]
  pub fn is_ok(self) -> bool
  {
    self == Self::Ok
  }
}

// Open and close a lock on the Mutex to find the state without using another thread
#[macro_export] macro_rules! state
{
  () =>
  {
    {
      /*
       * There is no need to drop the Mutex because it will auto-drop as
       * soon as the macro goes out-of-scope
       */
      *$crate::state::INIT_STATE.lock().unwrap()
    }
  };
}
pub use crate::state as state;

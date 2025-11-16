//! Init state management

use std::sync::Mutex;
use crate::display_enum;

#[derive(PartialEq, Eq, Clone, Copy, Debug, Default)]
#[must_use]
pub enum InitState { #[default] Ok = 0x02, Emergency = 0xA8, Stalled = 0xDC, Down = 0x20 }

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
    use InitState::*;

    const OK: u8 = InitState::Ok as u8;
    const EMERGENCY: u8 = InitState::Emergency as u8;
    const STALLED: u8 = InitState::Stalled as u8;

    match (byte) { OK => Ok, EMERGENCY => Emergency, STALLED => Stalled, _ => Down }
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

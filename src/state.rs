//! Init state management
// TO-DO: This is currently quite messy and potentially unneccessary

use std::sync::Mutex;

#[derive(PartialEq, Eq, Clone, Copy, Debug, Default, derive_more::Display)]
#[repr(u8)]
#[must_use]
pub enum InitState
{
  #[default]
  Okay = 0x02,
  Emergency = 0xa8,
  Stalled = 0xdc,
  Down = 0x20
}

// Setup a global mutex for the state, this can change once locked
pub static INIT_STATE: Mutex<InitState> = Mutex::new(InitState::Okay);

impl From<u8> for InitState
{
  fn from(byte: u8) -> Self
  {
    use InitState::{Okay, Emergency, Stalled, Down};

    const OKAY: u8 = Okay as u8;
    const EMERGENCY: u8 = Emergency as u8;
    const STALLED: u8 = Stalled as u8;

    match (byte)
    {
      OKAY => Okay,
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
    self == Self::Okay
  }
}

// Open and close a lock on the Mutex to find the state without using another thread
#[macro_export]
macro_rules! state
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

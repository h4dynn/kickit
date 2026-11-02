/*!
 * Provide service configuration & fields in a globally accessible cache file
 */

use std::{io::Write, sync::OnceLock, process::ExitStatus};
use crate::init::console::prelude::*;
use super::{RootService, ServiceBody, Pattern};

pub struct Cache
{
  pattern: Pattern,
  sandboxed: bool,
  code: Option<i32>,
  pid: Option<u32>
}

impl Cache
{
  pub const fn new(pattern: Pattern, sandboxed: bool, code: Option<i32>, pid: Option<u32>) -> Self
  {
    Self { pattern, sandboxed, code, pid }
  }

  pub fn as_data(&self) -> [u8; 11]
  {
    macro_rules! copy_array_slice
    {
      {$array: ident [$begin: literal .. $end: literal] = $assign: expr;} =>
      {
        for (idx, assign_idx) in (Iterator::zip($begin..$end, 0..$end - $begin))
        {
          $array [idx] = $assign [assign_idx];
        }
      };
    }

    let mut contents = [0u8; 11];

    (contents[0], contents[1]) = (self.pattern as u8, self.sandboxed.into());
    // Process ID (pid) is 4 bytes long as a u32
    copy_array_slice!{ contents[2..6] = self.pid.unwrap_or_default().to_le_bytes(); }

    if let Some(int_state) = self.code
    {
      // 0x01 indicates we have a status available - 0x00 indicates none available
      contents[6] = 0x01;
      // Exit code is 4 bytes long as an i32
      copy_array_slice!{ contents[7..11] = &int_state.to_le_bytes(); }
    }
    else {
      // Pad the remainder with null bytes for size consistency
      contents[7..11].fill_with(|| 0);
    }

    contents
  }

  /**
    * Output the service cache in its portable binary form
    *
    * # Errors
    *
    * The `out` buffer must be writable for 11 bytes.
    */
  /**
    * Generate configuration cache from the source file for ktctl to use. The reason why we cache instead of
    * just telling ktctl where the service's configuration file is, is to make sure ktctl is using the same
    * instance that the init is using, in case the service's config is changed at any point
    */
  pub fn write_as_data(self, out: &mut impl Write) -> Result<()>
  {
    out.write_all(&self.as_data()).into_trace(Error::ServiceAccess)?;

    Ok(())
  }
}

impl RootService
{
  /// Construct the cache instance by copying service fields
  pub(crate) fn cache(&self) -> Cache
  {
    use ServiceBody::{RunOnce, Standard};

    Cache::new(self.cfg.pattern, self.cfg.sandbox.is_some(),
                  // Include the exit status for RunOnce services
                  pat_layer_tree!(&self.body => RunOnce { code } => |Some| OnceLock::get => |Some| ExitStatus::code),
                  // Include active child process ID for standard services
                  pat_layer_tree!(&self.body => Standard { pid } => |Some| OnceLock::get).copied())
  }
}

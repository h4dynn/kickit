/*!
 * Mount implementation layered over the `nix` crate
 */

use crate::{BoxedStr, OptionizeEmpty, console::ErrorResult, init::console::prelude::*};
use std::{fmt, fmt::Display, path::{Path, PathBuf}, io};

/// Filesystem-specific options in string form
#[derive(PartialEq, Eq, Clone, Debug, Default)]
#[repr(transparent)]
pub struct Opts(Vec<BoxedStr>);

/// Idiomatic representation of a Linux mount entry
/**
  * # Abstract
  *
  * A structure of source, destination, filesystem, [flags] and [opts].
  *
  * This may represent an existing mount (from `/proc/mounts`) or an entry that hasn't been
  * mounted yet, which can be initialized with the [`mount`] method.
  *
  * # Linux (and other Unix-like)
  *
  * Mount entries are represented as a whitespace-delimited list (like you would find in `/etc/fstab`
  * or `/proc/mounts`). 
  *
  * # Construction
  *
  * ## Single
  *
  * The [`parse`] method constructs an entry from a whitespace-delimited list. If you already have
  * the values, see the [`new`] method.
  *
  * ## Multiple entry
  *
  * Mount entries are provided in the [`MountTable`] type (a transparent wrapper of a vector of
  * [`MountEntry`]).
  *
  * * To access the system fstab, use the [`fstab`] function.
  *
  * * To access the system mounts, use the [`mounttab`] function.
  *
  * * If you have a custom configuration file to parse, see [`MountTable::try_from_config_file`].
  *
  * [flags]: Flags
  * [opts]: Opts
  * [`mount`]: #method.mount
  * [`parse`]: Self::parse
  * [`new`]: Self::new
  */
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct MountEntry
{
  source: Option<PathBuf>,
  dest: PathBuf,
  fs: Option<BoxedStr>,
  flags: Flags,
  opts: Opts
}

/// Vector collection of mount entries, from a configuration or a list of already mounted entries
#[derive(PartialEq, Eq, Clone, Debug, Default)]
#[repr(transparent)]
pub struct MountTable(Vec<MountEntry>);

/**
  * A unique identifier will usually be displayed in its string form, but can be more
  * efficiently represented on the backend as 16 bytes
  */
#[derive(PartialEq, Eq, Copy, Clone, Debug, Default)]
#[repr(transparent)]
pub struct Uuid([u8; 16]);

/// All potential mappings used by Linux for a device (not including ID or diskseq)
#[derive(PartialEq, Eq, Clone)]
pub enum DeviceMap
{
  /// LABEL=<...>
  Label(BoxedStr),
  /// PARTLABEL=<...>
  PartLabel(BoxedStr),
  /// UUID=<...>
  Uuid(Uuid),
  /// PARTUUID=<...>
  PartUuid(Uuid)
}

c_style_flags! {
  /// Mount flags & their corresponding bit value (`MsFlags`)
  #[derive(PartialEq, Eq, Clone, Copy, Debug, derive_more::Display)]
  pub enum Flag: u64
  {
    /// The default behavior is to mount in read-write mode unless this flag is provided
    #[display("ro")]
    ReadOnly = 1,
    /// Forbid files with setuid attributes for security
    #[display("nosuid")]
    NoSuid = 2,
    /// Forbid special device files for security
    #[display("nodev")]
    NoDev = 4,
    /// Ignore execution access permissions on files, forbidding any execution from the filesystem
    #[display("noexec")]
    NoExec = 8,
    /// Remount a filesystem that is already mounted, allows new flags to be used
    #[display("remount")]
    Remount = 32,
    /// Bind a file/directory to another location, mirroring its contents & metadata
    #[display("bind")]
    Bind = 4096,
    /// Use a new namespace for this mount, restricting its propagation abilities
    #[display("private")]
    Private = 1 << 18
  }

  #[derive(PartialEq, Eq, Copy, Clone, Debug, Default)]
  pub struct Flags([Flag] as u64);
}

c_style_flags! {
  /// Flags used exclusively when unmounting a device
  #[derive(PartialEq, Eq, Clone, Copy, Debug)]
  pub enum UnmountFlag: i32
  {
    /// Unmount even if busy, may be dangerous (nfs only)
    Force = 1,
    /// Ignore checks & unmount anyway
    Lazy = 2,
    /// Mark the mountpoint as expired
    Expire = 4,
    /// Don't follow symlinks to their source
    NoFollow = 8
  }

  /// For optional use with `unmount`, which in turn calls `nix::mount::umount2`
  #[derive(PartialEq, Eq, Copy, Clone, Debug, Default)]
  pub struct UnmountFlags([UnmountFlag] as i32);
}

// Dereference to the first & only item in the tuple
wrap! {
  // Options specific to a filesystem type, will be concatenated, delimited by commas
  impl (Deref<Target = Vec<BoxedStr>>, DerefMut) for Opts;

  impl (Deref<Target = Vec<MountEntry>>, DerefMut) for MountTable;
}

/// Mount a device using [`nix::mount::mount`]
/**
  * # Errors
  *
  * * Couldn't mount the device (see `nix::mount::mount`)
  */
pub fn mount(src: Option<&str>, dest: &str, fs: Option<&str>, flags: Flags, opts: Option<&Opts>) -> Result<()>
{
  nix::mount::mount(src, dest, fs, flags.into(), opts.map(|s| s.join(",")).as_deref())
    .into_trace(Error::SysFsMount)
}

/// Unmount a device using [`nix::mount::umount`] or [`nix::mount::umount2`] if [`UnmountFlags`] are provided
/**
  * # Errors
  *
  * * Couldn't unmount the device (see `nix::mount::umount2` and `nix::mount::umount`)
  */
pub fn unmount(dest: &str, flags: Option<UnmountFlags>) -> Result<()>
{
  flags.map_or(nix::mount::umount(dest), |f| nix::mount::umount2(dest, f.into())).into_trace(Error::Unmount)
}

/**
  * # Errors
  *
  * * Couldn't read from /proc/mounts
  */
// Check if a path is a mountpoint
pub fn mounted(mountpoint: impl AsRef<Path> + Display) -> Result<bool>
{
  use std::fs::read_to_string;

  // Cycle through potential matches from /proc/mounts
  for candidate in (read_to_string("/proc/mounts").into_trace(Error::FileNotFound)?.split('\n'))
  {
    if (candidate.split(' ').nth(1) == Some(&mountpoint.to_string()))
    {
      return Ok(true)
    }
  }

  // Didn't find mount
  Ok(false)
}

/// Collect all the provided entries from the fstab (`/etc/fstab`)
/**
  * # Errors
  *
  * Refer to [`MountTable::try_from_config_file`] for more information.
  */
pub fn fstab() -> Result<MountTable>
{
  MountTable::try_from_config_file("/etc/fstab")
}

/// Collect all currently mounted devices from mounttab (`/proc/mounts`)
/**
  * # Errors
  *
  * Refer to [`MountTable::try_from_config_file`] for more information.
  */
pub fn mounttab() -> Result<MountTable>
{
  MountTable::try_from_config_file("/proc/mounts")
}

impl MountEntry
{
  /*
   * Construction methods
   */

  #[must_use]
  pub const fn new(source: Option<PathBuf>, dest: PathBuf, fs: Option<BoxedStr>, flags: Flags, opts: Opts) -> Self
  {
    Self { source, dest, fs, flags, opts }
  }

  /// Try parse whitespace-delimited string input of a mount entry
  /**
    *
    * **Note:** This method doesn't utilize the freq & passno values (5th and 6th),
    * but expects them to be required. Majority of the time these are both 0s.
    *
    * Example entries you would expect to see in /etc/fstab or /proc/mounts:
    *
    * ```text
    * /dev/source_device     /path/to/dest   fs_type   flags,and,options 0 0
    * LABEL=label_of_src_dev /path/to/dest_2 fs_type_2 flags,and,options 0 0
    * ```
    *
    * Note that /proc/mounts always has the exact location of the source device-
    * prefixed with /dev/
    *
    * # Errors
    *
    * * Entry doesn't have 6 elements (source, dest, fs, flags & opts, freq, passno)
    *
    * * Invalid provided device map (valid example: `PARTLABEL=label_of_gpt_part`)
    */
  pub fn parse(str_entry: &str) -> Result<Self>
  {
    use crate::guard;
    use DeviceMap::{Label, PartLabel, Uuid, PartUuid};

    // Split up by spaces, so we can parse each entry's information
    let entry: Vec<&str> = str_entry.split_ascii_whitespace().collect();
    // Although the last 2 values aren't used we still expect them for consistency with Linux standards
    guard!(entry.len() != 6 => Error::MountParse.trace("expected a line with 6 elements"));

    let dest = PathBuf::from(entry[1]);
    let source = (!matches!(entry[0], "auto" | "none")).then_some(if let Some((lookup, id)) = entry[0].split_once('=')
    {
      match (lookup)
      {
        // Construct `Uuid` type from string traditional-format UUID (see `<Uuid as TryFrom<&str>>::try_from`)
        "PARTUUID" => PartUuid(id.try_into().map_err(|e| Error::MountParse.trace(e))?),
        "UUID" => Uuid(id.try_into().map_err(|e| Error::MountParse.trace(e))?),
        // Label requires no type transformations, just a String (potentially a size-check could be added)
        "PARTLABEL" => PartLabel(id.into()), "LABEL" => Label(id.into()),
        _ => return Err(Error::MountParse.trace(format!("unknown map lookup type: {lookup}")))
      }
        .resolve()?.into()
    }
    else {
      // This is not a mapping, but a hard path to a source (e.g. /dev/nvme0n1p2, /dev/sda2, /dev/mmcblk0p1, ..)
      entry[0].into()
    });
    // auto|none is equivilant to Option::None here
    let fs = (!matches!(entry[2], "auto" | "none")).then_some(entry[2].into());
    let (flags, opts) =
    {
      // Flags - global settings for all filesystems no matter the type
      let mut flags = Flags::default();
      // Options - filesystem-type exclusive options
      let mut opts = Opts::default();

      // Read each flag in its string form (e.g. ReadOnly = "ro"), seperated by commas
      for option in (entry[3].split(','))
      {
        if let Ok(flag) = Flag::try_from(option)
        {
          // Add the flag as its u64
          flags.0 += flag as u64;
        }
        // Ignore the "defaults" flag - it isn't recognised by the libc mount call
        else if (option != "defaults")
        {
          (*opts).push(option.into());
        }
      }

      if (*dest == *"/")
      {
        // The rootfs will already be mounted so we need the remount flag
        flags.0 += Flag::Remount as u64;
      }

      // `.empty_none()` turns an vector into a Option, where if its empty None is produced
      (flags, opts)
    };

    Ok(Self { source, dest, fs, flags, opts })
  }

  /// Source device in entry, where `none` or `auto` become optionized
  #[must_use]
  pub fn source(&self) -> Option<&PathBuf>
  {
    self.source.as_ref()
  }

  /// Mount destination (file or directory)
  #[must_use]
  pub fn dest(&self) -> &PathBuf
  { 
    &self.dest
  }

  /// Initializes this mount, this method is a frontend to [`nix::mount::mount`]
  /**
    * # Errors
    *
    * Refer to [`nix::mount::mount`].
    */
  pub fn mount(&self) -> Result<()>
  {
    // Convert from `Flags` type to `nix::mount::MsFlags` and convert options into comma-delimited string
    let (src, dest, fs, flags, opts) = (self.source.as_ref(), &self.dest, self.fs.as_deref(),
                                            self.flags.into(), self.opts.join(","));

    // This method only accepts references to the values
    nix::mount::mount(src, dest, fs, flags, opts.empty_none()).into_trace(Error::SysFsMount)
  }

  /// Unmount this entry if already mounted, this method is a frontend to [`nix::mount::umount2`]
  /**
    * # Errors
    *
    * Refer to [`nix::mount::umount2`]
    */
  pub fn unmount(&self, flags: Option<UnmountFlags>) -> Result<()>
  {
    let dest = self.dest();

    flags.map_or(nix::mount::umount(dest), |f| nix::mount::umount2(dest, f.into())).into_trace(Error::Unmount)
  }
}

impl MountTable
{
  /// Construct mount table from traditional configuration file
  /**
    * # Errors
    *
    * This method expects the provided config file exists, is readable and contains valid formatted
    * entries. See [`MountEntry::parse`] for parsing failure information.
    */
  pub fn try_from_config_file(path: impl AsRef<Path>) -> Result<Self>
  {
    use std::{io::{BufReader, BufRead}, fs::File};

    let mut entries = Vec::new();

    // Open the configuration file for reading
    let config_file = File::open(&path).into_trace(Error::FileNotFound).subject(path.as_ref().display())?;

    for entry in (BufReader::new(config_file).lines())
    {
      if let Ok(sample) = &entry
      {
        // Ignore comments or empty lines
        continueif! (sample.starts_with('#') || sample.is_empty());
      }
      entries.push(MountEntry::parse(&entry.into_trace(Error::Unknown)?)?);
    }

    Ok(MountTable(entries))
  }
}

impl DeviceMap
{
  /**
    * Represent value as a formatted string
    *
    * # Example
    *
    * ```
    * let root_gpt = DeviceMap::PartUuid(Uuid::try_from("a05ef6ce-e186-415f-bf90-d70f6566ea6f").unwrap());
    * let efi_gpt = DeviceMap::Label(BoxedStr::from("EFI"));
    *
    * assert_eq!(&root_gpt.as_fmt(), "part-uuid = a05ef6ce-e186-415f-bf90-d70f6566ea6f");
    * assert_eq!(&efi_gpt.as_fmt(), "label = EFI");
    * ```
    */
  #[must_use]
  pub fn as_fmt(&self) -> String
  {
    match (self)
    {
      Self::Label(label) => ["label = ", label].concat(),
      Self::PartLabel(label) => ["part-label = ", label].concat(),
      Self::Uuid(uuid) => ["uuid = ", &uuid.as_string() as &str].concat(),
      Self::PartUuid(uuid) => ["part-uuid = ", &uuid.as_string() as &str].concat()
    }
  }

  /// Represent value in its traditional format (e.g. used in bash mount)
  #[must_use]
  pub fn as_raw(&self) -> String
  {
    match (self)
    {
      Self::Label(label) => ["LABEL=", label].concat(),
      Self::PartLabel(label) => ["PARTLABEL=", label].concat(),
      Self::Uuid(uuid) => ["UUID=", &uuid.as_string() as &str].concat(),
      Self::PartUuid(uuid) => ["PARTUUID=", &uuid.as_string() as &str].concat()
    }
  }

  /**
    * # Errors
    *
    * * All possible patterns for the device map were exhausted without a match
   **/
  pub fn resolve(self) -> Result<String>
  {
    let raw = self.as_fmt();

    /*
     * All possible matches of the mapping type, this is because sometimes UUIDs can be uppercase (like for NTFS),
     * or lowercase (majority of times), so we want to match every possible pattern
     */
    let patterns = match (self)
    {
      // We can assume the user has provided the exact proper label here, unlike for the UUID
      Self::Label(label) => vec![("by-label", label.to_string())],
      Self::PartLabel(label) => vec![("by-partlabel", label.to_string())],
      // Cover all possible UUID cases
      Self::Uuid(uuid) =>
      {
        let regular = uuid.as_string();
        let (lower, upper) = (regular.to_ascii_lowercase(), regular.to_ascii_uppercase());
        vec![("by-uuid", regular), ("by-uuid", lower), ("by-uuid", upper)]
      },
      Self::PartUuid(uuid) =>
      {
        let regular = uuid.as_string();
        let (lower, upper) = (regular.to_ascii_lowercase(), regular.to_ascii_uppercase());
        vec![("by-partuuid", regular), ("by-partuuid", lower), ("by-partuuid", upper)]
      }
    };

    for pattern in (patterns)
    {
      let path = format!("/dev/disk/{}/{}", pattern.0, pattern.1);

      // Check if this map exists, and if so follow it back to its source
      if (PathBuf::from(&path).is_symlink())
      {
        return Ok(path)
      }
    }

    Err(Error::Unknown.trace(format!("Lookup {raw} failed, no target was found!")))
  }
}

impl Uuid
{
  /// Initialize a value from existing byte form
  #[must_use]
  pub const fn new(inner: [u8; 16]) -> Self
  {
    Self(inner)
  }

  /// Return raw UUID in byte form
  #[must_use]
  pub fn as_inner(&self) -> &[u8; 16]
  {
    &self.0
  }

  /// Display UUID as its traditionally shown (example: `d60a475a-86aa-4588-94d4-b63af23b9084`)
  #[must_use]
  pub fn as_string(&self) -> String
  {
    let mut out = String::new();

    for slice in (self.as_parts())
    {
      for int in slice
      {
        // Format the byte as hexadecimal without prefixing with '0x'
        out.push_str(format!("{int:02x}").as_str());
      }
      out.push('-');
    }

    out
  }

  fn as_parts(&self) -> [&[u8]; 5]
  {
    [&self.0[0..4], &self.0[4..6], &self.0[6..8], &self.0[8..10], &self.0[10..16]]
  }
}

impl Opts
{
  #[must_use]
  pub const fn new(inner: Vec<BoxedStr>) -> Self
  {
    Self(inner)
  }
}

impl IntoIterator for MountTable
{
  type Item = MountEntry;
  type IntoIter = <Vec<MountEntry> as IntoIterator>::IntoIter;

  fn into_iter(self) -> Self::IntoIter
  {
    self.0.into_iter()
  }
}

impl From<Flags> for nix::mount::MsFlags
{
  fn from(flags: Flags) -> Self
  {
    Self::from_bits_retain(flags.0)
  }
}

impl From<UnmountFlags> for nix::mount::MntFlags
{
  fn from(flags: UnmountFlags) -> Self
  {
    Self::from_bits_retain(flags.0)
  }
}

impl TryFrom<&str> for Flag
{
  type Error = ErrorTrace;

  fn try_from(flag: &str) -> Result<Flag>
  {
    use Flag::{ReadOnly, NoSuid, NoDev, NoExec, Remount, Bind, Private};

    Ok(
      match (flag)
      {
        "ro" => ReadOnly, "nosuid" => NoSuid, "nodev" => NoDev, "noexec" => NoExec,
        "remount" => Remount, "bind" => Bind, "private" => Private,
        _ => Err(Error::Unknown.trace(format!("Unrecognised mount flag: {flag}")))?
      }
    )
  }
}

impl Display for Opts
{
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result
  {
    let mut stringOpts = String::new();

    for option in (&self.0)
    {
      stringOpts.push_str(option);
      // Options are seperated by commas
      stringOpts.push(',');
    }

    if (!stringOpts.is_empty())
    {
      // Remove the end trailing comma if it's there
      write!(f, "{}", stringOpts.strip_suffix(',').expect("Mount options formatting error"))?;
    }
    Ok(())
  }
}

// Convert from string representation of the UUID to its raw form
impl TryFrom<&str> for Uuid
{
  type Error = io::Error;

  fn try_from(string_uuid: &str) -> io::Result<Self>
  {
    use io::ErrorKind::InvalidInput;

    // Validate each section of the provided UUID is the exact length it should be
    macro_rules! affirm_section_len
    {
      ($parts: path [$($index: literal),*] .len() == [$($len: literal),*]) =>
      {
        $(
          if ($parts[$index].len() != $len)
          {
            return Err(io::Error::new(InvalidInput,
                          format!("invalid UUID: bad section length: {}", $parts[$index].len())))
          }
        )*
      };
    }

    let mut out = [0u8; 16];

    // UUID will be seperated by dashes - we don't care for these, they are simply for readability
    let parts_vec: Vec<&str> = string_uuid.split('-').collect();
    // There should be only 5 parts in the UUID, no more, no less
    let parts: [&str; 5] = parts_vec.try_into()
                              .map_err(|_| Self::Error::new(InvalidInput, "invalid UUID: expected 5 sections"))?;

    // This will be the exact length of each section of the UUID
    affirm_section_len!(parts[0, 1, 2, 3, 4].len() == [8, 4, 4, 4, 12]);

    // We have no reason to keep these parts seperate now, we just want to index them
    let final_uuid = parts.concat();
    // Decimal index is in bytes, so just increases by 1 but hex index is given as 2 characters, so its step is 2
    let (dec_index, hex_index) = (0..16, (0..32).step_by(2));

    // Index & hex index are zipped together so their range iteration is unified
    for (idx, hex_idx) in (Iterator::zip(dec_index, hex_index))
    {
      // Singular hex byte, which will be 2 characters in string form
      let hex = &final_uuid[hex_idx..hex_idx + 2];
      // Convert from the hexadecimal string into u8 byte using radix
      out[idx] = u8::from_str_radix(hex, 16)
                      .map_err(|e| Self::Error::new(InvalidInput, format!("invalid hexadecimal: {e}")))?;
    }

    Ok(Self(out))
  }
}

impl Display for Uuid
{
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result
  {
    write!(f, "{}", self.as_string())
  }
}

/// The construction for C-style flags is analogous so we use a macro to minimize boilerplate
#[macro_export]
macro_rules! c_style_flags_construct
{
  (<$ty: ty> :: from ([$($flag: ident),*]: [$item_ty: ty])) =>
  {{
    let mut flags = <$ty>::default();

    $(
      flags.push(<$item_ty>::$flag);
    )*

    flags
  }};
}

/// Helper macro akin to [`std::vec!`] to construct mount flags
#[macro_export(local_inner_macros)]
macro_rules! mount_flags
{
  [$($flag: ident),*] =>
  {
    c_style_flags_construct!(<$crate::init::mount::Flags>::from([$($flag),*]: [$crate::init::mount::Flag]))
  };
}
pub use mount_flags as flags;

/// Helper macro akin to [`std::vec!`] to construct unmount flags
#[macro_export(local_inner_macros)]
macro_rules! unmount_flags
{
  [$($flag: ident),*] =>
  {
    c_style_flags_construct!(<$crate::init::mount::UnmountFlags>::from([$($flag),*]:
                              [$crate::init::mount::UnmountFlag]))
  };
}
pub use unmount_flags as unmount_flags;

/// Construct array of filesystem-specific options (seperate from [`Flags`], which are global, regardless of fs type)
#[macro_export]
macro_rules! mount_opts
{
  // `$opt` type must be able to convert to a `Box<str>` (types that are include `&str` & `String`)
  [$($opt: tt),*] =>
  {
    <$crate::init::mount::Opts>::new(vec![$($opt.into()),*])
  };
}
pub use mount_opts as opts;

/// Construct a [`MountTable`] from input tuple
/**
  * # Syntax
  *
  * Optional values are specified with `_` for [`None`].
  *
  * # Example
  *
  * ```
  * # use kickit::init::mount::{MountTable, mount_table};
  * let custom_tab = mount_table!
  * [
  *   // Remount the root as read-write
  *   (_,                "/",         _,       [Private, Remount],               []),
  *   // btrfs mount with custom filesystem-specific options
  *   ("/dev/nvme0n1p2", "/mnt",      "btrfs", [Private],                        ["subvol=@", "compress=zstd:9"]),
  *   // Mount ESP
  *   ("/dev/sda",       "/boot/efi", "vfat",  [Private, NoDev, NoSuid, NoExec], [])
  * ];
  * ```
  */
#[macro_export]
macro_rules! mount_table
{
  [
    $(
      ($src: tt, $dest: expr, $fs_type: tt, [$($flag: ident),*], [$($opt: expr),*])
    ),+
  ] =>
  {
    {
      use $crate::init::mount::{MountTable, MountEntry, flags, opts, mount_table as option};

      let mut out = MountTable::default();

      $(
        out.push(MountEntry::new(option!(@$src), $dest.into(), option!(@$fs_type), flags![$($flag),*],
                                    opts![$($opt),*]));
      )+

      out
    }
  };
  // Parse optional value
  (@ _) => { None };
  (@ $some: tt) => { Some($some.into()) };
}
pub use mount_table as mount_table;

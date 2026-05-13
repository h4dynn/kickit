//! CLI interface for managing the init system

#![allow(unused_parens)]
#![allow(non_snake_case)]

extern crate chrono;

use std::{fs, fs::File, path::PathBuf, ffi::OsString, fmt};
use kickit::{console::{HandleError, Colour},
              ktctl::ktctl_console::{KTCtlErrorTrace, ConvKTCtlError, KTCtlError},
              console::affirm, display_enum, binary, path, state::InitState, Data};

// Dummy structure
struct Init;

// Lookup service from the runfs directory
#[derive(PartialEq, Eq, Clone, Debug)]
struct Service
{
  name: String
}

trait RunOperation
{
  fn root(&self) -> bool;
  fn initOnly(&self) -> bool;

  #[inline]
  async fn sanity(&self) -> Result<(), KTCtlErrorTrace>
  {
    use nix::unistd::getuid;

    if (self.root())
    {
      // Make sure we are root user
      affirm!(getuid().is_root(), KTCtlErrorTrace::new(KTCtlError::BadPerms, ""));
    }

    if (self.initOnly())
    {
      // Needs kickit to be ran
      affirm!(Init::is_running().await, KTCtlErrorTrace::new(KTCtlError::InitNotRunning, ""));
    }

    Ok(())
  }

  async fn run(self) -> Result<(), KTCtlErrorTrace>;
}

#[derive(PartialEq, Eq, Clone, Debug)]
#[must_use]
enum Operation
{
  Help(Option<String>), Version, ServiceList(Option<Vec<String>>),
  ServiceRestart(String), State, Log(String, bool, bool),
  TargetInfo, Shutdown, Reboot
}

trait SocketRequest: Sized
{
  // Whether we need root access for this socket or not
  const IS_PRIVATE: bool;

  /*
   * Provide a matching input byte for the socket request, this is what we
   * will write to the socket to send the request, and receive output data
   */
  fn in_byte(&self) -> u8;

  // The name for the socket we access
  fn name(&self) -> String;

  async fn request(self) -> Result<Data, KTCtlErrorTrace>
  {
    use tokio::{net::UnixStream, io::AsyncReadExt};

    // Determine where the socket is that we want to interact with
    let path = if (Self::IS_PRIVATE)
    {
      format!("/run/kickit/private/io.{}", self.name())
    }
    else {
      format!("/run/kickit/io.{}", self.name())
    };

    // Open a new connection to the socket, may fail if there is an existing connection
    let mut io = UnixStream::connect(&path).await.context_trace(&path, KTCtlError::SocketAccessFail)?;
    let mut out = Data::new();

    io.writable().await.trace(KTCtlError::SocketAccessFail)?;
    io.try_write(&[self.in_byte()]).trace(KTCtlError::AccessRunFsFail)?;
    io.readable().await.trace(KTCtlError::SocketAccessFail)?;
    io.read_to_end(&mut out).await.context_trace("io.Core", KTCtlError::SocketAccessFail)?;

    // An 0x0f byte means the operation failed
    if (out.as_slice() == [0x0f])
    {
      Err(KTCtlErrorTrace::new(KTCtlError::SocketAccessFail,
            &format!("Error returned by init after requesting {}", self.name())))
    }
    else {
      Ok(out)
    }
  }
}

// Possible socket requests, like e.g. core or power
mod Socket
{
  use kickit::socket;

  #[derive(PartialEq, Eq, Copy, Clone, Debug)]
  pub(super) enum Core
  {
    State = socket::Core::STATE as isize,
    Version = socket::Core::VERSION as isize,
    Target = socket::Core::TARGET as isize,
    Pid = socket::Core::PID as isize
  }

  #[derive(PartialEq, Eq, Copy, Clone, Debug)]
  pub(super) enum Power
  {
    Shutdown = socket::Power::SHUTDOWN as isize,
    Reboot = socket::Power::REBOOT as isize
  }

  #[derive(Copy, Clone, Debug)]
  pub struct Log;

  /*
   * This is just boilerplate implementation, so we use a macro to
   * make it look less ugly
   */
  macro_rules! impl_SocketRequest
  {
    { for $($name: ty { private = $private: tt }),* } =>
    {
      $(
        impl super::SocketRequest for $name
        {
          const IS_PRIVATE: bool = $private;

          fn in_byte(&self) -> u8
          {
            *self as u8
          }
          fn name(&self) -> String
          {
            self.to_string()
          }
        }
      )*
    };
    { for $($name: ty = $byte: path { private = $private: tt }),* } =>
    {
      $(
        impl super::SocketRequest for $name
        {
          const IS_PRIVATE: bool = $private;

          fn in_byte(&self) -> u8
          {
            $byte
          }
          fn name(&self) -> String
          {
            self.to_string()
          }
        }
      )*
    };
  }

  impl_SocketRequest!
  {
    for Core { private = false },
    Power { private = true }
  }
  impl_SocketRequest!
  {
    for Log = socket::Log::MASTER { private = true }
  }
}

#[derive(PartialEq, Eq, Clone, Copy, Debug, Default)]
enum Usage
{
  #[doc = include_str!("../../docs/ktctl_usage/main.txt")]
  #[default]
  Main,

  #[doc = include_str!("../../docs/ktctl_usage/log.txt")]
  Log,

  #[doc = include_str!("../../docs/ktctl_usage/service.txt")]
  Service,

  Taskitty
}

display_enum!
{
  Socket::Core { _ => "Core" },
  Socket::Power { _ => "Power" }
}
display_enum!
{
  // Show the usage prompts for the help operation
  Usage {
    Main =>
    {
      format!("{}{b}{}{r}{}\n{}\n{}{b}{}{r}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}{}\n{}\n",
        "Usage: ", binary!(), " [OPERATION]",
        "Manage the kickit init system",
        '\n',
        "Operations:",
        " help                     Show this help prompt",
        " version                  Show kickit version",
        " service [S]              List all services or selected services",
        " log [S]                  Read a service's logs",
        " state                    Show current init state",
        " target                   Show current loaded target",
        " shutdown                 Shutdown this device",
        " reboot                   Reboot this device",
        '\n',
        "Try 'ktctl help [OPERATION]' for more info",
        b = Colour::BOLD, r = Colour::RESET
      )
    },
    Log =>
    {
      format!("{}{}{b}{}{r}{}\n{}\n{}{b}{}{r}\n{}\n{}\n{}\n",
        "Usage: ", binary!(), " log", " [ARGUMENTs..] [SERVICE]",
        "View a service's or init's logs (requires root access)",
        '\n',
        "Arguments:",
        " --plain              Plain output (no colours + timestamp as millis)",
        " --init               View the init's master log",
        " --service-only       Ignore any messages from init",
        b = Colour::BOLD, r = Colour::RESET
      )
    },
    Service => format!("Usage: {}{} service{} [NAMEs..]", binary!(), Colour::BOLD, Colour::RESET),
    // her name is taskitty ^.^
    Taskitty => include_str!("../../assets/taskitty.txt").to_string()
  }
}

impl RunOperation for Operation
{
  fn root(&self) -> bool
  {
    matches!(self, Self::Log(..) | Self::Shutdown | Self::Reboot)
  }

  fn initOnly(&self) -> bool
  {
    matches!(self, Self::ServiceList(..) | Self::TargetInfo | Self::Log(..) |
                    Self::Shutdown | Self::Reboot)
  }

  async fn run(self) -> Result<(), KTCtlErrorTrace>
  {
    use Operation::{Help, Version, ServiceList, ServiceRestart, State, TargetInfo,
                  Log, Shutdown, Reboot};

    self.sanity().await?;

    match (self)
    {
      Help(..) => self.help(), Version => { Self::version(); Ok(()) },
      ServiceList(s) => Self::serviceList(s.as_deref()),
      ServiceRestart(s) => Self::serviceRestart(s), State => Init::prettyState().await,
      TargetInfo => Self::targetInfo().await, Log(s, u, i) => Self::readLog(&s as &str, u, i).await,
      Shutdown => Self::shutdown().await, Reboot => Self::reboot().await
    }
  }
}

impl fmt::Display for Socket::Log
{
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result
  {
    write!(f, "Log")
  }
}

impl TryFrom<&str> for Usage
{
  type Error = ();

  fn try_from(str_flag: &str) -> Result<Self, ()>
  {
    match (str_flag)
    {
      "" => Ok(Usage::Main), "log" => Ok(Usage::Log),
      "service" => Ok(Usage::Service), "purr" => Ok(Usage::Taskitty),
      _ => Err(())
    }
  }
}

impl Init
{
  pub async fn state() -> InitState
  {
    use InitState::{Down, Emergency, Stalled};

    if let Ok(state) = Socket::Core::State.request().await
    {
      // Convert the state from a u8 byte to an InitState
      match (state[0].into())
      {
        InitState::Ok =>
        {
          /*
           * Make sure the version of `ktctl` and `kickit` match to avoid
           * potential compatibility issues
           */
          if let Ok(version) = Socket::Core::Version.request().await &&
              (version.as_slice() == format!("{}\n", kickit::VERSION).as_bytes())
          {
            return InitState::Ok
          }
        },
        // No further checks need to be done here
        Emergency | Stalled => return state[0].into(),
        // Falls down to the end of the function
        Down => ()
      }
    }
    Down
  }

  // Print the state in a pretty way
  #[inline]
  pub async fn prettyState() -> Result<(), KTCtlErrorTrace>
  {
    use Socket::Core::Pid;
    use InitState::{Emergency, Down, Stalled, Ok};

    let state = Self::state().await;
    // Matching colour for our init state (e.g. up / running = green)
    let colour = match (state)
    {
      Emergency | Down => Colour::RED, Stalled => Colour::ORANGE, Ok => Colour::GREEN
    };

    let initPid = u32::from_be_bytes(Pid.request().await?
                                      .try_into()
                                      .map_err(|_| KTCtlErrorTrace::new(KTCtlError::Format, "Invalid init pid!"))?);

    if (initPid > 1)
    {
      println!("{colour}{state}{} (pid: {initPid})", Colour::RESET);
    }
    else {
      println!("{colour}{state}{}", Colour::RESET);
    }

    Result::Ok(())
  }

  // Check if init is running
  pub async fn is_running() -> bool
  {
    Self::state().await != InitState::Down
  }
}

impl From<OsString> for Service
{
  // Display our OsString first and then use that Display to be converted to a String
  fn from(name: OsString) -> Self
  {
    Self { name: name.display().to_string() }
  }
}

/*
 * (slight annoyance): we can't use a generic AsRef<str> because of way too strict
 * Rust compilation rules:
 *   upstream crates may add a new impl of trait `std::convert::AsRef<str>` for type
 *   `std::ffi::OsString` in future versions
 */
impl From<&str> for Service
{
  fn from(name: &str) -> Self
  {
    Self { name: name.to_owned() }
  }
}

impl Service
{
  pub fn is_standard(&self) -> bool
  {
    fs::metadata(path!("/run/kickit/service/", &self.name, "pid")).is_ok()
  }

  pub fn pid(&self) -> Result<Option<String>, KTCtlErrorTrace>
  {
    if (self.is_standard())
    {
      let pid = fs::read_to_string(path!("/run/kickit/service", &self.name, "pid"))
                  .trace(KTCtlError::AccessRunFsFail)?;

      Ok(Some(pid))
    }
    else {
      Ok(None)
    }
  }

  #[inline]
  pub fn path(&self, pathName: &str) -> Result<String, KTCtlErrorTrace>
  {
    if (pathName == "stat")
    {
      // This file should exist or else its a bug
      Ok(format!("/proc/{}/stat", self.pid()?.expect("path(stat): pid is missing")))
    }
    else {
      panic!()
    }
  }

  pub fn print(self) -> Result<(), KTCtlErrorTrace>
  {
    let status: String =
    [
      String::from(
      {
        if (self.is_standard())
        {
          // Standard services will contain more information (e.g. pid)
          "├─ Status: "
        }
        else {
          // Non-standard services will just have a status & nothing else
          "└─ Status: "
        }
      }),
      if (self.is_standard())
      {
        // Read the /proc/<PID>/stat file, which contains the process's status information
        if let Ok(stat) = fs::read_to_string(self.path("stat")?)
        {
          // The 3rd member (split by spaces) contains the status we want (e.g. I = idle)
          match (stat.split(' ').nth(2))
          {
            // Z = zombie (stopped running) and X = killed (by another process)
            Some("Z" | "X") => format!("{}Dead{}", Colour::RED, Colour::RESET),
            // These are all acceptable process statuses
            Some("S" | "I" | "D" | "R") => format!("{}Up{}", Colour::GREEN, Colour::RESET),
            /*
             * This might happen sometimes, for example on older kernel versions which
             * may have additional signals which are now removed / deprecated
             */
            Some(..) | None => format!("{}Unknown{}", Colour::RED, Colour::RESET)
          }
        }
        else {
          format!("{}Dead{}", Colour::RED, Colour::RESET)
        }
      }
      /*
       * If the `/run/kickit/service/<S>/killed` file exists, we know the service
       * process exited on an OK signal (0)
       */
      else if (fs::metadata(path!("/run/kickit/service/", &self.name, "exited")).is_ok())
      {
        format!("{}Finished{}", Colour::GREEN, Colour::RESET)
      }
      // If not, it exited on a failure (non-zero)
      else {
        format!("{}Failed{}", Colour::RED, Colour::RESET)
      }
    ]
      .join("");

    println!("{}{}{}", Colour::BOLD, self.name, Colour::RESET);
    println!("{status}");

    // Non-standard services will not have an active PID
    if (self.is_standard())
    {
      println!("└─ PID:    {}", self.pid()?.unwrap());
    }

    // Print newline seperator for next service
    println!();

    Ok(())
  }
}

impl Operation
{
  #[inline]
  fn help(self) -> Result<(), KTCtlErrorTrace>
  {
    // Check if user provided an operation to look at or not
    println!("{}", if let Self::Help(Some(name)) = self
    {
      // Check if the name has a matching usage doc to print
      if let Ok(usage) = Usage::try_from(&name as &str)
      {
        usage
      }
      else {
        // Return here, deviate from our default way of printing
        return match (&name as &str)
        {
          // Generate a generic usage prompt here since there is no extra info available
          "help" | "version" | "state" | "target" =>
          {
            println!("Usage: {} {}{}{}", binary!(), Colour::BOLD, name, Colour::RESET);
            Ok(())
          },
          // Unrecognised operation provided
          _ => Err(KTCtlErrorTrace::new(KTCtlError::InvalidOperation, &name))
        }
      }
    }
    else {
      Usage::Main
    });

    Ok(())
  }

  #[inline]
  fn version()
  {
    println!("{b}{}{r}\n\n{b}ktctl:{r} {}", kickit::ktctl::LOGO, kickit::PRETTY_VERSION(),
                                            b = Colour::BOLD, r = Colour::RESET);
  }

  fn serviceList(maybeTargets: Option<&[String]>) -> Result<(), KTCtlErrorTrace>
  {
    for service in (fs::read_dir(PathBuf::from("/run/kickit/service"))
                      .trace(KTCtlError::RunFsParseFail)?)
    {
      // Import each entry as a service from its OsString file name
      let upService: Service = service.trace(KTCtlError::AccessRunFsFail)?.file_name().into();

      // Only print if targets provided by user allow
      if let Some(targets) = maybeTargets
      {
        if (targets.contains(&upService.name))
        {
          upService.print()?;
        }
      }
      else {
        // Print all the services
        upService.print()?;
      }
    }

    Ok(())
  }

  fn serviceRestart(_service: String) -> Result<(), KTCtlErrorTrace>
  {
    todo!();
  }

  async fn targetInfo() -> Result<(), KTCtlErrorTrace>
  {
    // Read target name from the socket, and format as a String
    eprint!("{}", String::from_utf8(Socket::Core::Target.request().await?)
                      .context_trace("target", KTCtlError::Format)?);
    Ok(())
  }

  async fn shutdown() -> Result<(), KTCtlErrorTrace>
  {
    use std::{thread, time::Duration};
    Socket::Power::Shutdown.request().await.map(|_| thread::sleep(Duration::new(u64::MAX, 0)))
  }

  async fn reboot() -> Result<(), KTCtlErrorTrace>
  {
    use std::{thread, time::Duration};
    Socket::Power::Reboot.request().await.map(|_| thread::sleep(Duration::new(u64::MAX, 0)))
  }

  async fn readLog(serviceName: &str, ugly: bool, ignoreInit: bool) -> Result<(), KTCtlErrorTrace>
  {
    use chrono::{Local, DateTime};
    use std::{io, io::{BufWriter, Read, Write, ErrorKind::BrokenPipe}, process::exit};
    use ruzstd::decoding::StreamingDecoder;

    // Not a service, we want to read the init's logs
    if (serviceName == "init")
    {
      eprintln!("{}", String::from_utf8(Socket::Log.request().await?).trace(KTCtlError::Format)?);
      return Ok(())
    }

    // The decoder implements Read to idiomatically decompress
    let mut serviceLog = StreamingDecoder::new(File::open(path!("/run/kickit/service", &serviceName, "log"))
                                            .context_trace(serviceName, KTCtlError::BadService)?)
                          .context_trace(serviceName, KTCtlError::LogAccessFail)?;

    let mut logBin = Vec::new();
    // Read (potentially binary) log contents
    serviceLog.read_to_end(&mut logBin).trace(KTCtlError::AccessRunFsFail)?;

    // Our stdout that we write to
    let mut out = BufWriter::new(io::stdout());

    // Log properties for every line
    let mut timestamp = String::new();
    let mut logContents = String::new();
    let mut fromInit = false;

    // Loop through each byte in the log
    for (logByteCount, logByte) in (logBin.iter().enumerate())
    {
      match (logByte)
      {
        b'\n' =>
        {
          // We don't want to show an empty line if it's found (which it shouldn't be)
          if !(logContents.is_empty() || fromInit && ignoreInit)
          {
            let marker = if (fromInit)
            {
              if (ugly)
              {
                String::from("(kickit) ")
              }
              else {
                format!("{}(kickit){} ", Colour::BOLD, Colour::RESET)
              }
            }
            else {
              String::new()
            };

            let lineFormatted =
            {
              if (ugly)
              {
                // Don't make the timestamp human-readable, just millis
                format!("[{timestamp}] {marker}{logContents}\n")
              }
              else {
                // Convert the millis type from a String to i64 so it is accepted by chrono
                let timestampUgly: i64 = timestamp.parse().trace(KTCtlError::Format)?;

                /*
                 * Get the timestamp from the log and convert it into an actual date & time,
                 * then convert from UTC timezone to the system's timzone (Local) using
                 * the chrono crate magic
                 */
                let logTime: DateTime<Local> = DateTime::from_timestamp_millis(timestampUgly)
                                                .context_trace(serviceName, KTCtlError::LogAccessFail)?
                                                .into();

                /*
                 * Format time as <Day Month Year, Hours:Minutes:Seconds> to not
                 * anger the Americans
                 */
                format!("[{}] {marker}{logContents}\n", logTime.format("%d %b %Y, %H:%M:%S"))
              }
            };

            /*
             * When we output to stdout and another program is piped (e.g. grep),
             * the program cuts off the pipe after it has consumed everything it
             * needs so we end up with SIGPIPE. We don't error here if found, since
             * this is expected & not an actual error, though is treated as one
             * by println!() so we have to do a janky workaround for compatibility
             *
             * (note): This may change in the future though, see
             * <https://github.com/rust-lang/rust/issues/62569>
             */
            out.write_all(lineFormatted.as_bytes())
                .map_err(|e| if (e.kind() == BrokenPipe) { exit(0) } else { e })
                .trace(KTCtlError::Format)?;
          }

          // Reset our values for next line
          timestamp.clear();
          logContents.clear();
          fromInit = false;
        },
        // This is the marker that the message is from the init
        0x8F =>
        {
          // Must be the 14th byte or else something is wrong
          affirm!(timestamp.len() == 13 && logContents.is_empty(),
            KTCtlErrorTrace::with_context(KTCtlError::Format, &serviceName,
              &format!("Unexpected byte 0x8F on byte {logByteCount}")));

          fromInit = true;
        },
        _ =>
        {
          // The timestamp is just 13 bytes long
          if (timestamp.len() < 13)
          {
            // We are not done reading the full timestamp
            timestamp.push(*logByte as char);
          }
          else {
            // This is real log content, not a timestamp
            logContents.push(*logByte as char);
          }
        }
      }
    }

    Ok(())
  }
}

// Handle user arguments, including the operation & targets for it
fn parseArgs(arguments: &[String]) -> Result<Operation, KTCtlErrorTrace>
{
  use Operation::{Help, Version, TargetInfo, State, ServiceList, ServiceRestart, Log, Shutdown, Reboot};

  if (arguments.len() == 1)
  {
    return Ok(Help(None))
  }

  Ok(match (&arguments[1] as &str)
  {
    "-h" | "--help" => Help(None),
    "version" => Version,
    "target" => TargetInfo,
    "state" => State,
    "help" =>
    {
      if (arguments.len() == 2)
      {
        // Regular help prompt
        Help(None)
      }
      else {
        // Help prompt about a specific operation
        Help(Some(arguments[2].clone()))
      }
    },
    "service" =>
    {
      if (arguments.len() == 2)
      {
        ServiceList(None)
      }
      else {
        ServiceList(Some(arguments[2..].into()))
      }
    },
    "service-restart" =>
    {
      if (arguments.len() == 3)
      {
        ServiceRestart(arguments[2].clone())
      }
      else {
        Err(KTCtlErrorTrace::new(KTCtlError::MissingArgument, "service-restart"))?
      }
    },
    "log" =>
    {
      // Must be at least log + service name
      affirm!(arguments.len() > 2, KTCtlErrorTrace::new(KTCtlError::MissingArgument, "log"));

      let args = &arguments[2..];

      // Be pretty by default unless --plain is given
      let ugly: bool = args.iter().any(|arg| arg == "--plain");
      // Don't ignore init by default unless --service-only is given
      let ignoreInit: bool = args.iter().any(|arg| arg == "--service-only");

      let mut argIter = args.iter();

      let serviceName =
      {
        if (args.iter().any(|arg| arg == "--init"))
        {
          // Not a service, just display stuff
          String::from("init")
        }
        else {
          // Can't use for loop because you can't break in one for some reason
          loop {
            match (argIter.next())
            {
              // Check for argument with a valid potential name
              Some(argument) =>
              {
                if (!argument.starts_with("--"))
                {
                  break Ok(argument)
                }
              },
              // Cycled through all possible arguments without a matching name
              None => break Err(KTCtlErrorTrace::new(KTCtlError::MissingArgument, "log"))
            }
          }?.to_owned()
        }
      };
      Log(serviceName, ugly, ignoreInit)
    },
    "shutdown" => Shutdown,
    "reboot" => Reboot,
    // Unknown operation
    _ => Err(KTCtlErrorTrace::new(KTCtlError::InvalidOperation, &arguments[1]))?
  })
}

#[tokio::main]
async fn main()
{
  use std::env;

  // Extract operation from our arguments or throw error
  let operation = parseArgs(&env::args().collect::<Vec<String>>()).handle();

  operation.run().await.handle();
}

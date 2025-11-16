//! CLI interface for managing the init system

#![allow(unused_parens)]
#![allow(non_snake_case)]

extern crate chrono;

use std::{fs, fs::File, path::PathBuf, ffi::OsString, fmt::Display};
use kickit::{console::{HandleKTError, Colour}, ktctl::ktctl_console::*,
              affirm, display_enum, binary, path, state::InitState, Data};

#[derive(PartialEq, Eq, Clone, Debug)]
#[must_use]
enum Operation { Help(Option<String>), Version, ServiceList(Vec<String>),
                  State, Log(String, bool, bool), TargetInfo,
                  // The `bool`s in shutdown/reboot are for forcing or not
                  Shutdown(bool), Reboot(bool) }

#[derive(PartialEq, Eq, Clone, Copy, Debug, Default)]
enum Usage
{
  #[doc = include_str!("../../docs/ktctl_usage/main.txt")] #[default] Main,
  #[doc = include_str!("../../docs/ktctl_usage/log.txt")] Log,
  #[doc = include_str!("../../docs/ktctl_usage/service.txt")] Service,
  #[doc = include_str!("../../docs/ktctl_usage/shutdown.txt")] Shutdown,
  #[doc = include_str!("../../docs/ktctl_usage/reboot.txt")] Reboot,
  Taskitty
}

struct Init;
#[derive(PartialEq, Eq, Clone, Debug)]
struct Service { name: String }

// Possible socket requests, like e.g. core or power
mod SocketRequest
{
  use kickit::socket;

  /*
   * Provide a matching input byte for the socket request, this is what we
   * will write to the socket to send the request, and receive output data
   */
  pub(super) trait Byte { fn in_byte(self) -> u8; }

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

  /*
   * This is just boilerplate implementation, so we use a macro to
   * make it look less ugly
   */
  macro_rules! impl_Byte
  {
    { for $($name: ty),* } =>
    {
      $(impl Byte for $name { fn in_byte(self) -> u8 { self as u8 } })*
    }
  }

  impl_Byte! { for Core, Power }
}

display_enum! { SocketRequest::Core }
display_enum!
{
  // Show the usage prompts for the help operation
  Usage {
    Main =>
      format!("{}{b}{}{r}{}\n{}\n\n{b}{}{r}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n\n{}\n",
              "Usage: ", binary!(), " [OPERATION]",
              "Manage the kickit init system",
              "Operations:",
              " help                  Show this help prompt",
              " version               Show kickit version",
              " service [S]           List all services or selected services",
              " state                 Show current init state",
              " log [S]               Read a service's logs",
              " target                Show current loaded target",
              " shutdown [--force]    Shutdown this device",
              " reboot [--force]      Reboot this device",
              "Try 'ktctl help [OPERATION]' for more info",
              b = Colour::BOLD, r = Colour::RESET),

    Log => format!("{}{}{b}{}{r}{}\n{}\n\n{b}{}{r}\n{}\n{}\n",
                  "Usage: ", binary!(), " log", " [ARGUMENTs..] [SERVICE]",
                  "View a service's logs (requires root access)",
                  "Arguments:",
                  " --plain              Plain output (no colours + timestamp as millis)",
                  " --service-only       Ignore any messages from init",
                  b = Colour::BOLD, r = Colour::RESET),

    Service => format!("Usage: {}{} service{} [NAMEs..]", binary!(), Colour::BOLD, Colour::RESET),
    Shutdown => format!("Usage: {}{} shutdown{} [--force]", binary!(), Colour::BOLD, Colour::RESET),
    Reboot => format!("Usage: {}{} reboot{} [--force]", binary!(), Colour::BOLD, Colour::RESET),

    // her name is taskitty ^.^
    Taskitty => include_str!("../ascii_art/taskitty.txt").to_string()
  }
}

impl Default for Operation { fn default() -> Self { Self::Help(None) } }

impl From<&str> for Usage
{
  fn from(str_flag: &str) -> Self
  {
    match (str_flag) { "" => Usage::Main, "log" => Usage::Log, "service" => Usage::Service,
                        "shutdown" => Usage::Shutdown, "reboot" => Usage::Reboot,
                        "purr" => Usage::Taskitty,
                        e => panic!("Unrecognised flag passed when converting &str to Usage: {e}") }
  }
}

impl Usage
{
  // Check if a string is a valid variant of usage
  fn valid(w: &str) -> bool { matches!(w, "" | "log" | "service" | "shutdown" | "reboot" | "purr") }
}

fn readSocket<R: SocketRequest::Byte + Display + Copy>(req: R) -> Result<Data, KTCtlErrorTrace>
{
  use std::{os::unix::net::UnixStream, io::{Read, Write}};

  let mut io = UnixStream::connect("/run/kickit/io.Core")
                      .context_trace("io.Core", KTCtlError::SocketAccessFail)?;
  let mut out = Data::new();

  io.write_all(&[req.in_byte()]).trace(KTCtlError::AccessRunFsFail)?;
  io.read_to_end(&mut out).context_trace("io.Core", KTCtlError::SocketAccessFail)?;

  // An 0x0f byte means the operation failed
  if (out.as_slice() == [0x0f])
  {
    Err(KTCtlErrorTrace::new(KTCtlError::SocketAccessFail,
        format!("Error returned by init after requesting {req}")))
  }
  else {
    Ok(out)
  }
}

impl Init
{
  pub fn state() -> InitState
  {
    if let Ok(state) = readSocket(SocketRequest::Core::State)
    {
      match (state[0].into())
      {
        InitState::Emergency | InitState::Stalled => return state[0].into(),
        InitState::Ok =>
        {
          if let Ok(s) = readSocket(SocketRequest::Core::Version) &&
            (s == format!("{}\n", kickit::VERSION).as_bytes())
          {
            return InitState::Ok
          }
        }
        _ => ()
      }
    }
    InitState::Down
  }

  // Print the state in a pretty way
  #[inline]
  pub fn prettyState() -> Result<(), KTCtlErrorTrace>
  {
    use InitState::*;

    let state = Self::state();
    let colour = match (state)
    {
      Emergency | Down => Colour::RED,
      Stalled => Colour::ORANGE,
      Ok => Colour::GREEN
    };
    let addon = if (state == Ok)
    {
      let initPid = u32::from_be_bytes(match (readSocket(SocketRequest::Core::Pid)?.try_into())
      {
        Result::Ok(s) => Result::Ok(s),
        Err(..) => Err(KTCtlErrorTrace::new(KTCtlError::FormatFail, "Invalid init pid!"))
      }?);

      if (initPid != 1)
      {
        format!(" (pid: {initPid})")
      }
      else {
        String::new()
      }
    }
    else {
      String::new()
    };

    // Find a matching colour for the state (e.g. red for down)
    println!("{colour}{state}{}{addon}", Colour::RESET);
    Result::Ok(())
  }

  // Check if init is running
  pub fn is_running() -> bool { Self::state() == InitState::Ok }
}

impl From<&str> for Service { fn from(name: &str) -> Self { Self { name: name.into() } } }

impl From<OsString> for Service
{
  // Display our OsString first and then use that Display to be converted to a String
  fn from(name: OsString) -> Self { Self { name: name.display().to_string() } }
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

  #[inline] pub fn path(&self, pathName: &str) -> Result<String, KTCtlErrorTrace>
  {
    match (pathName)
    {
      "stat" => Ok(format!("/proc/{}/stat", self.pid()?.expect("path(stat): pid is missing"))),
      _ => panic!()
    }
  }

  pub fn print(self) -> Result<(), KTCtlErrorTrace>
  {
    let statusHead = String::from(if (self.is_standard()) { "├─ Status: " } else { "└─ Status: " });

    let status: String =
    [
      statusHead,
      if (self.is_standard())
      {
        if let Ok(stat) = fs::read_to_string(self.path("stat")?)
        {
          match (stat.split(" ").nth(2))
          {
            Some("Z" | "X") => format!("{}Dead", Colour::RED),
            Some("S" | "I" | "D" | "R") => format!("{}Up", Colour::GREEN),
            // This might happen sometimes because there are other unimplemented types
            Some(..) | None => format!("{}Unknown", Colour::RED)
          }
        }
        else {
          format!("{}Dead", Colour::RED)
        }
      }
      else if (fs::metadata(path!("/run/kickit/service/", &self.name, "exited")).is_ok())
      {
        format!("{}Finished", Colour::GREEN)
      }
      else {
        format!("{}Failed", Colour::RED)
      },
      Colour::RESET.to_string()
    ]
      .join("");

    println!("{}{}{}", Colour::BOLD, self.name, Colour::RESET);
    println!("{status}");

    if (self.is_standard()) { println!("└─ PID:    {}", self.pid()?.unwrap()) }

    println!();

    Ok(())
  }
}

impl Operation
{
  fn services(targets: Vec<String>) -> Result<(), KTCtlErrorTrace>
  {
    for serviceEntry in (fs::read_dir(PathBuf::from("/run/kickit/service"))
                          .trace(KTCtlError::RunFsParseFail)?)
    {
      // Import each entry as a service from its OsString file name
      let upService: Service = serviceEntry.trace(KTCtlError::AccessRunFsFail)?.file_name().into();

      // Only print if targets allow
      if (targets.is_empty() || (targets.contains(&upService.name)))
      {
        upService.print()?;
      }
    }

    Ok(())
  }

  fn readLog(serviceName: String, ugly: bool, ignoreInit: bool) -> Result<(), KTCtlErrorTrace>
  {
    use chrono::{Local, DateTime};
    use std::{io, io::{BufReader, BufWriter, Write, ErrorKind}};
    use zstd::stream::decode_all as zstdDecompressFile;

    let serviceLog = File::open(path!("/run/kickit/service", &serviceName, "log"))
                      .context_trace(&serviceName, KTCtlError::BadService)?;

    // Read (potentially binary) log contents
    let logBin = zstdDecompressFile(BufReader::new(serviceLog)).trace(KTCtlError::AccessRunFsFail)?;

    // Our stdout that we write to
    let mut out = BufWriter::new(io::stdout());

    // Log properties for every line
    let mut timestamp = String::new();
    let mut logContents = String::new();
    let mut fromInit = false;

    // Loop through each line in the log
    for logChar in (logBin)
    {
      match (logChar)
      {
        // 0x0a is a newline byte
        0x0A =>
        {
          // We don't want to show an empty line if it's found (which it shouldn't be)
          if !(logContents.is_empty() || fromInit && ignoreInit)
          {
            let marker = if (fromInit)
            {
              [ if (!ugly) { format!("{}", Colour::BOLD) } else { String::new() },
                format!("(kickit){} ", Colour::RESET) ].concat()
            }
            else {
              String::new()
            };

            let lineFmt = if (!ugly)
            {
              // Convert the millis type from a String to i64 so it is accepted by chrono
              let timestampUgly: i64 = timestamp.parse().trace(KTCtlError::FormatFail)?;

              /*
               * Get the timestamp from the log and convert it into an actual date & time,
               * then convert from UTC timezone to the system's timzone (Local)
               */
              let logTime: DateTime<Local> = DateTime::from_timestamp_millis(timestampUgly)
                                            .context_trace(&serviceName, KTCtlError::LogAccessFail)?
                                            .into();

              // Format time as <Day Month Year, Hours:Minutes:Seconds>
              format!("[{}] {marker}{logContents}\n", logTime.format("%d %b %Y, %H:%M:%S"))
            }
            else {
              // Don't make the timestamp human-readable, just millis
              format!("[{timestamp}] {marker}{logContents}\n")
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
            if let Err(error) = out.write_all(lineFmt.as_bytes())
            {
              // The expected "error" in question: SIGPIPE
              if (error.kind() == ErrorKind::BrokenPipe)
              {
                std::process::exit(0);
              }

              // An *actual* error was found
              Err(error).trace(KTCtlError::FormatFail)?;
            }
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
              KTCtlErrorTrace::with_context(KTCtlError::FormatFail, serviceName,
                                            "Unexpected byte 0x8F"));

          fromInit = true;
        },
        _ =>
        {
          // The timestamp is just 13 bytes long
          if (timestamp.len() < 13)
          {
            timestamp.push(logChar as char);
          }
          else {
            logContents.push(logChar as char);
          }
        }
      }
    }

    Ok(())
  }

  fn getTarget() -> Result<(), KTCtlErrorTrace>
  {
    eprint!("{}", String::from_utf8(readSocket(SocketRequest::Core::Target)?)
                              .context_trace("target", KTCtlError::FormatFail)?);
    Ok(())
  }

  fn shutdown(_force: bool) -> !
  {
    use kickit::console::ReturnError;
    use nix::sys::reboot::{reboot, RebootMode::RB_POWER_OFF};

    reboot(RB_POWER_OFF).trace(KTCtlError::AccessRunFsFail).unwrap_err().fatal();
  }

  fn reboot(_force: bool) -> !
  {
    todo!();
  }

  #[inline] fn usage(operation: Option<String>) -> Result<(), KTCtlErrorTrace>
  {
    // Check if user provided an operation to look at or not
    println!("{}", if let Some(ref name) = operation
    {
      if (Usage::valid(name as &str)) { name.as_str().into() }
      else {
        return match (name as &str)
        {
          "help" | "version" | "state" | "target" =>
          {
            println!("Usage: {} {}{}{}", binary!(), Colour::BOLD, name, Colour::RESET);
            Ok(())
          },
          _ => Err(KTCtlErrorTrace::new(KTCtlError::InvalidOperation, name))
        }
      }
    }
    else { Usage::Main });

    Ok(())
  }

  #[inline] fn version()
  {
    use kickit::ktctl::LOGO;

    println!("{}{}{}\n", Colour::BOLD, LOGO, Colour::RESET);
    println!("{}ktctl:{} {}", Colour::BOLD, Colour::RESET, kickit::version());
  }

  #[inline] fn sanity(&self) -> Result<(), KTCtlErrorTrace>
  {
    use nix::unistd::getuid;

    if (matches!(self, Operation::Log(..) | Operation::Shutdown(..) | Operation::Reboot(..)))
    {
      affirm!(getuid().is_root(),
        KTCtlErrorTrace::new(KTCtlError::BadPerms, ""));
    }

    if (matches!(self, Operation::TargetInfo | Operation::ServiceList(..) | Operation::Log(..) |
                       Operation::Shutdown(..) | Operation::Reboot(..)))
    {
      affirm!(Init::is_running(),
        KTCtlErrorTrace::new(KTCtlError::InitNotRunning, String::from("Init process not found")));
    }

    Ok(())
  }

  #[inline]
  pub fn run(self) -> Result<(), KTCtlErrorTrace>
  {
    use Operation::*;
    match (self)
    {
      Help(o) => Ok(Self::usage(o)?), Version => { Self::version(); Ok(()) },
      TargetInfo => Ok(Self::getTarget()?), ServiceList(service) => Ok(Self::services(service)?),
      State => { Init::prettyState()?; Ok(()) },
      Log(service, ugly, ignoreInit) => Ok(Self::readLog(service, ugly, ignoreInit)?),
      Shutdown(force) => Self::shutdown(force), Reboot(force) => Self::reboot(force)
    }
  }
}

fn cliArguments(arguments: Vec<String>) -> Result<Operation, KTCtlErrorTrace>
{
  if (arguments.len() == 1) { return Ok(Operation::default()) }

  match (&arguments[1] as &str)
  {
    "-h" | "--help" => Ok(Operation::Help(None)),
    "version" => Ok(Operation::Version),
    "target" => Ok(Operation::TargetInfo),
    "state" => Ok(Operation::State),
    "help" => Ok(if (arguments.len() == 2) { Operation::Help(None) }
                  else { Operation::Help(Some(arguments[2].to_string())) }
              ),
    "service" => Ok(if (arguments.len() == 2) { Operation::ServiceList(Vec::new()) }
                    else { Operation::ServiceList(arguments[2..].into()) }
              ),
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

      let serviceName: String = loop
      {
        match (argIter.next())
        {
          // Check for argument with a valid potential name
          Some(x) => { if (!x.starts_with("--")) { break Ok(x) } },
          // Cycled all through possible arguments without a matching name
          None => break Err(KTCtlErrorTrace::new(KTCtlError::MissingArgument, "log"))
        }
      }?
        .to_owned();

      Ok(Operation::Log(serviceName, ugly, ignoreInit))
    },
    "shutdown" => Ok(Operation::Shutdown(arguments.len() > 2 && &arguments[2] == "--force")),
    "reboot" => Ok(Operation::Reboot(arguments.len() > 2 && &arguments[2] == "--force")),
    // Unknown operation
    _ => Err(KTCtlErrorTrace::new(KTCtlError::InvalidOperation, &arguments[1]))
  }
}

fn main()
{
  use std::env;

  let operation = cliArguments(env::args().collect()).handle();

  // Check that the operation can be safely ran before continuing
  operation.sanity().handle();

  operation.run().handle();
}

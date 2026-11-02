//! CLI interface for managing the init system

#![allow(unused_parens)]
#![allow(non_snake_case)]

extern crate chrono;

use std::fmt;
use kickit::{console::prelude::*, socket::{Core, Log, Power}, ktctl::{console::prelude::*, socket::Request},
              console::{guard, ErrorResult}, binary, state::InitState};

// Dummy structure
struct Init;

trait RunOperation
{
  // Requires root access to run
  fn root(&self) -> bool;
  // Requires the init system to run
  fn initOnly(&self) -> bool;

  #[inline]
  async fn sanity(&self) -> Result<()>
  {
    use nix::unistd::getuid;

    if (self.root())
    {
      // Make sure we are root user
      guard!(!getuid().is_root() => Error::OperationNotPermitted.into());
    }

    if (self.initOnly())
    {
      // Needs kickit to be ran as the init process
      guard!(!Init::is_running().await => Error::InitNotRunning.into());
    }

    Ok(())
  }

  async fn run(self) -> Result<()>;
}

#[derive(PartialEq, Eq, Clone, Debug)]
#[must_use]
enum Operation
{
  Help(Option<String>),
  Version,
  ServiceList(Option<Vec<String>>),
  State,
  Log(String, bool, bool),
  InitLog,
  TargetInfo,
  Shutdown(bool),
  Reboot(bool)
}

#[derive(PartialEq, Eq, Clone, Copy, Debug, Default)]
enum Usage
{
  #[default]
  Main,
  Log,
  Service,
  Taskitty
}

impl fmt::Display for Usage
{
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result
  {
    write!(f, "{}", match (self)
    {
      Self::Main =>
      {
        format!("{}{}{}\n{}\n{}{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}{}\n{}\n",
          "Usage: ", binary!().bold(), " [ACTION]",
          "Manage the kickit init system",
          '\n',
          "Actions:".bold(),
          " help                       Usage information for ktctl or one of its actions",
          " version                    Version of this ktctl",
          " service [SERVICEs]         List services & their fields",
          " log SERVICE                View a service's logs, or the init's master log",
          " state                      Current state of the init system",
          " target                     View the currently loaded target",
          " shutdown [--force]         Shut down system",
          " reboot [--force]           Reboot system",
          '\n',
          "Try 'ktctl help [ACTION]' for more info"
        )
      },
      Self::Log =>
      {
        format!("{}{}{}{}\n{}\n{}{}\n{}\n{}\n{}\n",
          "Usage: ", binary!().bold(), " log", " [ARGUMENTs..] [SERVICE]",
          "View a service's or init's logs (requires root access)",
          '\n',
          "Arguments:".bold(),
          " --plain              Plain output (no colours + timestamp as millis)",
          " --init               View the init's master log",
          " --service-only       Ignore any messages from init"
        )
      },
      Self::Service => format!("Usage: {} service [NAMEs..]", binary!().bold()),
      // ^.^
      Self::Taskitty => include_str!("../../assets/taskitty.txt").to_string()
    })
  }
}

impl TryFrom<&str> for Usage
{
  type Error = ();

  fn try_from(flag: &str) -> StdResult<Self, ()>
  {
    match (flag)
    {
      "" => Ok(Usage::Main),
      "log" => Ok(Usage::Log),
      "service" => Ok(Usage::Service),
      "purr" => Ok(Usage::Taskitty),
      _ => Err(())
    }
  }
}

impl Init
{
  pub async fn pid() -> Result<u32>
  {
    // Response should be 4 bytes (u32, in little endian byte order, so reverse)
    let bytes: [u8; 4] = Core.request(Core::PID).await??.try_into()
                            .map_err(|_| Error::Format.trace("invalid init pid!"))?;

    Ok(u32::from_le_bytes(bytes))
  }

  pub async fn state() -> InitState
  {
    // Make sure the init and ktctl are of the same version to avoid any potential compatiblity issues
    if (Core.request(Core::VERSION).await != Ok(Ok(format!("{}\n", env!("CARGO_PKG_VERSION")).into_bytes())))
    {
      return InitState::Down;
    }

    if let Ok(Ok(state)) = Core.request(Core::STATE).await
    {
      state[0].into()
    }
    else {
      InitState::Down
    }
  }

  // Print the state in a pretty way
  #[inline]
  pub async fn prettyState() -> Result<()>
  {
    use InitState::{Okay, Down, Stalled};

    let state = Self::state().await;
    // Matching colour for our init state (e.g. up / running = Green)
    let colour = match (state) { Okay => Colour::Green, Stalled => Colour::Orange, _ => Colour::Red };

    if (state == Down)
    {
      println!("{}", Down.colour(Colour::Red));
    }
    else {
      match (Init::pid().await?)
      {
        1 => println!("{}", state.colour(colour)),
        not_init => println!("{} (pid: {not_init})", state.colour(colour))
      }
    }

    Ok(())
  }

  // Check if init is running
  pub async fn is_running() -> bool
  {
    Self::state().await != InitState::Down
  }

  pub async fn readLog() -> Result<()>
  {
    eprintln!("{}", str::from_utf8(Log.request(Log::MASTER).await??.as_slice()).into_trace(Error::Format)?);
    Ok(())
  }
}

impl Operation
{
  #[inline]
  fn help(self) -> Result<()>
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
            println!("Usage: {} {name}", binary!().bold());
            Ok(())
          },
          // Unrecognised operation provided
          _ => Err(Error::InvalidOperation.trace(&name))
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
    use kickit::ktctl::LOGO;
    println!("{}\n{} {}", LOGO.bold(), "ktctl:".bold(), kickit::VERSION);
  }

  async fn targetInfo() -> Result<()>
  {
    // Read target name from the socket, and format as a String
    print!("{}", String::from_utf8(Core.request(Core::TARGET).await??).into_trace(Error::Format).subject("target")?);
    Ok(())
  }

  async fn shutdown(reboot: bool, force: bool) -> Result<()>
  {
    use std::thread::park;

    // The corresponding byte we will send to the socket
    let ask = match (reboot)
    {
      true if (force) => Power::FORCE_REBOOT,
      false if (force) => Power::FORCE_SHUTDOWN,
      true => Power::REBOOT,
      false => Power::SHUTDOWN
    };

    let pid = Init::pid().await?;

    // Send power signal
    if let Ok(Err(error)) = Power.request(ask).await
    {
      return Err(error.into());
    }

    if (pid == 1)
    {
      // Block until shutdown
      loop {
        park();
      }
    }
    // Should only reach this point if kickit wasn't the init process
    Ok(())
  }
}

impl RunOperation for Operation
{
  fn root(&self) -> bool
  {
    use Operation::{InitLog, Log, Shutdown, Reboot};
    matches!(self, InitLog | Log(..) | Shutdown(..) | Reboot(..))
  }

  fn initOnly(&self) -> bool
  {
    use Operation::{ServiceList, TargetInfo, InitLog, Log, Shutdown, Reboot};
    matches!(self, ServiceList(..) | TargetInfo | InitLog | Log(..) | Shutdown(..) | Reboot(..))
  }

  async fn run(self) -> Result<()>
  {
    use kickit::ktctl::service::{PartialService, serviceList};
    use Operation::{Help, Version, ServiceList, State, TargetInfo,
                  InitLog, Log, Shutdown, Reboot};

    // Check if root access / init is required for this operation
    self.sanity().await?;

    match (self)
    {
      Help(..) => self.help(),
      Version => { Self::version(); Ok(()) },
      ServiceList(services) => serviceList(services.as_deref()),
      Log(name, ugly, ignoreInit) => PartialService::import(&name)?.logs(ugly, ignoreInit),
      InitLog => Init::readLog().await,
      State => Init::prettyState().await,
      TargetInfo => Self::targetInfo().await,
      Shutdown(force) => Self::shutdown(false, force).await,
      Reboot(force) => Self::shutdown(true, force).await,
    }
  }
}

// Handle user arguments, including the operation & targets for it
fn parseArgs(mut arguments: Vec<String>) -> Result<Operation>
{
  use kickit::breakif;
  use Operation::{Help, Version, TargetInfo, State, ServiceList, InitLog, Log, Shutdown, Reboot};

  match (&arguments[0] as &str)
  {
    "shutdown" | "poweroff" => return Ok(Shutdown(false)),
    "reboot" => return Ok(Reboot(false)),
    _ => {
      if (arguments.len() == 1)
      {
        return Ok(Help(None))
      }
    }
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
        Help(Some(arguments.remove(2)))
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
    "log" =>
    {
      // Must provide at least service name
      guard!(arguments.len() < 2 => Error::MissingArgument.trace("log"));

      let args = &arguments[2..];

      // Be pretty by default unless --plain is given
      let ugly: bool = args.iter().any(|arg| arg == "--plain");
      // Don't ignore init by default unless --service-only is given
      let ignoreInit: bool = args.iter().any(|arg| arg == "--service-only");

      let mut argIter = args.iter();

      // Read the init master log (seperate implementation since it uses a socket)
      if (args.iter().any(|arg| arg == "--init"))
      {
        return Ok(InitLog)
      }

      let serviceName =
      {
        // Can't use for loop because you can't break in one for some reason
        loop {
          match (argIter.next())
          {
            // Check for argument with a valid potential name
            Some(argument) => breakif! (!argument.starts_with("--") => Ok(argument.clone())),
            // Cycled through all possible arguments without a matching name
            None => break Err(Error::MissingArgument.trace("log"))
          }
        }?
      };
      Log(serviceName, ugly, ignoreInit)
    },
    "shutdown" => Shutdown(arguments.iter().any(|arg| arg == "--force")),
    "reboot" => Reboot(arguments.iter().any(|arg| arg == "--force")),
    // Unknown operation
    _ => Err(Error::InvalidOperation.trace(arguments.remove(1)))?
  })
}

#[tokio::main]
async fn main()
{
  use std::env;

  // Extract operation from our arguments or throw error
  let args = env::args().collect::<Vec<String>>();
  let operation = parseArgs(args).handle();

  operation.run().await.handle();
}

//! CLI interface for managing the init system

#![allow(unused_parens)]
#![allow(non_snake_case)]

extern crate chrono;

use kickit::{console::{HandleError, Colour, ExtendWithContext},
              socket::{Core, Log, Power},
              ktctl::{console::{StdResult, Result, ConvError, Error}, socket::Request},
              console::affirm, display_enum, binary, state::InitState};

// Dummy structure
struct Init;

trait RunOperation
{
  fn root(&self) -> bool;
  fn initOnly(&self) -> bool;

  #[inline]
  async fn sanity(&self) -> Result<()>
  {
    use nix::unistd::getuid;

    if (self.root())
    {
      // Make sure we are root user
      affirm!(getuid().is_root(), Error::OperationNotPermitted.trace(""));
    }

    if (self.initOnly())
    {
      // Needs kickit to be ran
      affirm!(Init::is_running().await, Error::InitNotRunning.trace(""));
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
  ServiceRestart(String),
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
        " log (--init) <S>         Read a service's logs",
        " state                    Show current init state",
        " target                   Show current loaded target",
        " shutdown [--force]       Shutdown this device (force not recommended!)",
        " reboot [--force]         Reboot this device (force not recommended!)",
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
    // ^.^
    Taskitty => include_str!("../../assets/taskitty.txt").to_string()
  }
}

impl TryFrom<&str> for Usage
{
  type Error = ();

  fn try_from(strFlag: &str) -> StdResult<Self, ()>
  {
    match (strFlag)
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
  pub async fn state() -> InitState
  {
    use InitState::{Down, Emergency, Stalled};

    if let Ok(state) = Core.request(Core::STATE).await
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
          if let Ok(version) = Core.request(Core::VERSION).await &&
              (version.as_slice() == format!("{}\n", env!("CARGO_PKG_VERSION")).as_bytes())
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
  pub async fn prettyState() -> Result<()>
  {
    use InitState::{Emergency, Down, Stalled};

    let state = Self::state().await;
    // Matching colour for our init state (e.g. up / running = green)
    let colour = match (state)
    {
      Emergency | Down => Colour::RED,
      Stalled => Colour::ORANGE,
      InitState::Ok => Colour::GREEN
    };

    /*
     * Request the init PID from the socket, this is how we test if kickit
     * is running as the init process or not
     */
    let initPid = u32::from_le_bytes(Core.request(Core::PID).await?.try_into()
                                      .map_err(|_| Error::Format.trace("Invalid init pid!"))?);

    if (initPid == 1)
    {
      println!("{colour}{state}{}", Colour::RESET);
    }
    else {
      println!("{colour}{state}{} (pid: {initPid})", Colour::RESET);
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
    eprintln!("{}", String::from_utf8(Log.request(Log::MASTER).await?).into_trace(Error::Format)?);
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
            println!("Usage: {} {}{}{}", binary!(), Colour::BOLD, name, Colour::RESET);
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
    use kickit::{ktctl::LOGO, version};

    println!("{b}{}{r}\n{b}ktctl:{r} {}", LOGO, version(), b = Colour::BOLD, r = Colour::RESET);
  }

  async fn targetInfo() -> Result<()>
  {
    // Read target name from the socket, and format as a String
    eprint!("{}", String::from_utf8(Core.request(Core::TARGET).await?).into_trace(Error::Format).context("target")?);
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

    let noInit =
    {
      let response = Core.request(Core::NO_INIT).await?[0];
      match (response)
      {
        0 => Ok(false), 1 => Ok(true),
        // This really should never happen but is still a possibility
        _ => Err(Error::SocketResponse.trace(format!("Expected a boolean, got {response}")))
      }
    }?;
    // Send power signal
    let _ = Power.request(ask).await?;

    if (!noInit)
    {
      // Block until shutdown
      park();
    }
    // Should only reach this point if kickit wasn't the init process
    Ok(())
  }
}

impl RunOperation for Operation
{
  fn root(&self) -> bool
  {
    matches!(self, Self::InitLog | Self::Log(..) | Self::Shutdown(..) | Self::Reboot(..))
  }

  fn initOnly(&self) -> bool
  {
    matches!(self, Self::ServiceList(..) | Self::TargetInfo | Self::InitLog |
                    Self::Log(..) | Self::Shutdown(..) | Self::Reboot(..))
  }

  async fn run(self) -> Result<()>
  {
    use kickit::ktctl::service::{Service, serviceList};
    use Operation::{Help, Version, ServiceList, ServiceRestart, State, TargetInfo,
                  InitLog, Log, Shutdown, Reboot};

    // Check if root access / init is required for this operation
    self.sanity().await?;

    match (self)
    {
      Help(..) => self.help(),
      Version =>
      {
        Self::version();
        Ok(())
      },
      ServiceList(services) => serviceList(services.as_deref()),
      ServiceRestart(..) => todo!(), /*serviceRestart(services),*/
      Log(name, ugly, ignoreInit) =>
      {
        let service: Service = name.as_str().into();
        service.readLog(ugly, ignoreInit)
      },
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
  use Operation::{Help, Version, TargetInfo, State, ServiceList,
                    ServiceRestart, InitLog, Log, Shutdown, Reboot};

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
    "service-restart" =>
    {
      if (arguments.len() == 3)
      {
        ServiceRestart(arguments.remove(2))
      }
      else {
        Err(Error::MissingArgument.trace("service-restart"))?
      }
    },
    "log" =>
    {
      // Must be at least log + service name
      affirm!(arguments.len() > 2, Error::MissingArgument.trace("log"));

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

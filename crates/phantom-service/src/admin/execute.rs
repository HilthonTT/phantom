//! Commands the server runs at itself.
//!
//! Two lists, both from the config: `admin_execute` runs once the services are
//! up, and `admin_signal_execute` runs again every time the server is sent
//! `SIGUSR2`. Between them they are how an operator scripts the server without
//! a client — a fresh deployment can register its first account, and a running
//! one can be prodded from a unit file or a cron job.
//!
//! Output goes to the log rather than to the admin room. These commands have
//! no room and no event to answer, so the log is where their output belongs
//! even once the timeline write path lands.

use std::time::Duration;

use phantom_core::{Err, Result, debug, implement, info};
use tokio::time::sleep;

use super::CommandOutput;

pub(super) const SIGNAL: &str = "SIGUSR2";

/// How long the startup commands wait before running.
///
/// Every worker is spawned at once and there is nothing that announces one is
/// ready, so a command that reaches a service doing its setup in its own
/// worker — the emergency password, say — can arrive before that has happened.
/// This is the crude answer, and it goes away when the services broadcast
/// their run state.
const STARTUP_DELAY: Duration = Duration::from_millis(500);

/// Runs the `admin_execute` commands.
#[implement(super::Service)]
pub(super) async fn startup_execute(&self) -> Result {
    // Cloned rather than borrowed: the config manager hands out a reference
    // through a thread-local, and the loop below awaits, which may leave the
    // task on another thread than the one the reference came from.
    let commands = self.services.server.config.admin_execute.clone();
    if commands.is_empty() {
        return Ok(());
    }

    sleep(STARTUP_DELAY).await;

    self.execute_commands(&commands).await
}

/// Runs the `admin_signal_execute` commands.
#[implement(super::Service)]
pub(super) async fn signal_execute(&self) -> Result {
    let commands = self.services.server.config.admin_signal_execute.clone();

    self.execute_commands(&commands).await
}

#[implement(super::Service)]
async fn execute_commands(&self, commands: &[String]) -> Result {
    let ignore_errors = self.services.server.config.admin_execute_errors_ignore;

    for (i, command) in commands.iter().enumerate() {
        if let Err(e) = self.execute_command(i, command.clone()).await
            && !ignore_errors
        {
            return Err(e);
        }

        // These run back to back on the worker's task before it starts taking
        // commands from the queue, so it yields between them.
        tokio::task::yield_now().await;
    }

    Ok(())
}

#[implement(super::Service)]
async fn execute_command(&self, i: usize, command: String) -> Result {
    debug!("Execute command #{i}: executing {command:?}");

    // In place rather than queued: the queue is drained by the worker that is
    // running this, so queueing here would deadlock, and the caller wants the
    // outcome anyway.
    match self.command_in_place(command, None).await {
        Ok(None) => {
            info!("Execute command #{i} completed with no output.");
            Ok(())
        }
        Ok(Some(output)) => {
            info!("Execute command #{i} completed:\n{}", body(&output));
            Ok(())
        }
        Err(output) => Err!(error!("Execute command #{i} failed:\n{}", body(&output))),
    }
}

/// The text of a command's output.
///
/// Commands answer in markdown, since their usual reader is a Matrix client.
/// The plain body is what a log line wants, and it is the same text without
/// the formatting.
fn body(output: &CommandOutput) -> &str {
    output.body()
}

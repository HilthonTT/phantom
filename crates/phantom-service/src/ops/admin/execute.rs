use std::time::Duration;

use phantom_core::{Err, Result, debug, implement, info};
use tokio::time::sleep;

use super::CommandOutput;

pub(super) const SIGNAL: &str = "SIGUSR2";

const STARTUP_DELAY: Duration = Duration::from_millis(500);

#[implement(super::Service)]
pub(super) async fn startup_execute(&self) -> Result {
    let commands = self.services.server.config.admin.admin_execute.clone();
    if commands.is_empty() {
        return Ok(());
    }

    sleep(STARTUP_DELAY).await;

    self.execute_commands(&commands).await
}

#[implement(super::Service)]
pub(super) async fn signal_execute(&self) -> Result {
    let commands = self
        .services
        .server
        .config
        .admin
        .admin_signal_execute
        .clone();

    self.execute_commands(&commands).await
}

#[implement(super::Service)]
async fn execute_commands(&self, commands: &[String]) -> Result {
    let ignore_errors = self
        .services
        .server
        .config
        .admin
        .admin_execute_errors_ignore;

    for (i, command) in commands.iter().enumerate() {
        if let Err(e) = self.execute_command(i, command.clone()).await
            && !ignore_errors
        {
            return Err(e);
        }

        tokio::task::yield_now().await;
    }

    Ok(())
}

#[implement(super::Service)]
async fn execute_command(&self, i: usize, command: String) -> Result {
    debug!("Execute command #{i}: executing {command:?}");

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

fn body(output: &CommandOutput) -> &str {
    output.body()
}

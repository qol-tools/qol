mod model;

pub(crate) use model::{Command, ModifierKeys};

use crate::config::ServerConfig;
use crate::input::InputHandler;
use crate::network::bind_udp_or_inherit;
use crate::security::CommandGate;
use anyhow::Result;
use std::sync::Arc;

pub struct CommandService {
    input_handler: Arc<InputHandler>,
    security: Arc<CommandGate>,
}

impl CommandService {
    pub fn new(input_handler: InputHandler, security: Arc<CommandGate>) -> Result<Self> {
        Ok(Self {
            input_handler: Arc::new(input_handler),
            security,
        })
    }

    pub async fn run(&self) -> Result<()> {
        let socket = bind_udp_or_inherit("command", ServerConfig::COMMAND_PORT).await?;
        socket.set_broadcast(true)?;

        let mut buf = [0; ServerConfig::COMMAND_BUFFER_SIZE];

        loop {
            match socket.recv_from(&mut buf).await {
                Ok((size, _addr)) => {
                    let Ok(command) = self.security.authenticate(&buf[..size]) else {
                        continue;
                    };
                    let _ = self.input_handler.handle_command(command);
                }
                Err(e) => {
                    log::error!("Command receive error: {}", e);
                }
            }
        }
    }
}

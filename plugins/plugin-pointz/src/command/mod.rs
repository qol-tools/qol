mod model;

pub(crate) use model::{Command, ModifierKeys};

use crate::config::ServerConfig;
use crate::input::InputHandler;
use crate::network::bind_udp_or_inherit;
use anyhow::Result;
use std::sync::Arc;

pub struct CommandService {
    input_handler: Arc<InputHandler>,
}

impl CommandService {
    pub fn new(input_handler: InputHandler) -> Result<Self> {
        Ok(Self {
            input_handler: Arc::new(input_handler),
        })
    }

    pub async fn run(&self) -> Result<()> {
        let socket = bind_udp_or_inherit("command", ServerConfig::COMMAND_PORT).await?;
        socket.set_broadcast(true)?;

        let mut buf = [0; ServerConfig::COMMAND_BUFFER_SIZE];

        loop {
            match socket.recv_from(&mut buf).await {
                Ok((size, _addr)) => {
                    if let Ok(command) = serde_json::from_slice::<Command>(&buf[..size]) {
                        let _ = self.input_handler.handle_command(command);
                    }
                }
                Err(e) => {
                    log::error!("Command receive error: {}", e);
                }
            }
        }
    }
}

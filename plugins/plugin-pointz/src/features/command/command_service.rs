use anyhow::Result;
use std::net::UdpSocket;
use std::sync::Arc;
use crate::domain::models::Command;
use crate::domain::config::ServerConfig;
use crate::input::InputHandler;

/// Service that receives and processes commands from clients
pub struct CommandService {
    input_handler: Arc<InputHandler>,
}

impl CommandService {
    /// Creates a new CommandService
    pub fn new(input_handler: InputHandler) -> Result<Self> {
        Ok(Self {
            input_handler: Arc::new(input_handler),
        })
    }

    /// Runs the command loop in a blocking manner
    pub fn run_blocking(&self) -> Result<()> {
        let socket = UdpSocket::bind(format!("0.0.0.0:{}", ServerConfig::COMMAND_PORT))?;
        socket.set_broadcast(true)?;

        let mut buf = [0; ServerConfig::COMMAND_BUFFER_SIZE];

        loop {
            match socket.recv_from(&mut buf) {
                Ok((size, _addr)) => {
                    if let Ok(command) = serde_json::from_slice::<Command>(&buf[..size]) {
                        let handler = Arc::clone(&self.input_handler);
                        tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(async {
                                let _ = handler.handle_command(command).await;
                            })
                        });
                    }
                }
                Err(e) => {
                    log::error!("Command receive error: {}", e);
                }
            }
        }
    }
}


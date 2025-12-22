use anyhow::Result;
use tokio::net::UdpSocket;
use crate::domain::models::Command;
use crate::domain::config::ServerConfig;
use crate::input::InputHandler;

/// Service that receives and processes commands from clients
pub struct CommandService {
    socket: UdpSocket,
    input_handler: InputHandler,
}

impl CommandService {
    /// Creates a new CommandService bound to the command port
    pub async fn new(input_handler: InputHandler) -> Result<Self> {
        let socket = UdpSocket::bind(format!("0.0.0.0:{}", ServerConfig::COMMAND_PORT)).await?;
        socket.set_broadcast(true)?;
        Ok(Self {
            socket,
            input_handler,
        })
    }

    /// Runs the command loop, processing incoming commands indefinitely
    pub async fn run(&self) -> Result<()> {
        let mut buf = [0; ServerConfig::COMMAND_BUFFER_SIZE];
        
        loop {
            match self.socket.recv_from(&mut buf).await {
                Ok((size, _addr)) => {
                    if let Ok(command) = serde_json::from_slice::<Command>(&buf[..size]) {
                        if let Err(e) = self.input_handler.handle_command(command).await {
                            log::error!("Command error: {}", e);
                        }
                    }
                }
                Err(e) => {
                    log::error!("Command receive error: {}", e);
                }
            }
        }
    }
}


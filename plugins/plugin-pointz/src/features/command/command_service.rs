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
        let mut total_time = std::time::Duration::ZERO;
        let mut count = 0u32;

        loop {
            let loop_start = std::time::Instant::now();

            match self.socket.recv_from(&mut buf).await {
                Ok((size, _addr)) => {
                    let parse_start = std::time::Instant::now();
                    if let Ok(command) = serde_json::from_slice::<Command>(&buf[..size]) {
                        let parse_time = parse_start.elapsed();

                        let handle_start = std::time::Instant::now();
                        if let Err(e) = self.input_handler.handle_command(command).await {
                            log::error!("Command error: {}", e);
                        }
                        let handle_time = handle_start.elapsed();

                        let total = parse_time + handle_time;
                        if total.as_micros() > 10000 {
                            log::debug!("Slow command processing: parse={}µs handle={}µs total={}µs",
                                parse_time.as_micros(), handle_time.as_micros(), total.as_micros());
                        }

                        count += 1;
                        total_time += total;
                        if count % 100 == 0 {
                            log::info!("Avg processing time over 100 commands: {}µs", total_time.as_micros() / 100);
                            total_time = std::time::Duration::ZERO;
                        }
                    }
                }
                Err(e) => {
                    log::error!("Command receive error: {}", e);
                }
            }

            let loop_time = loop_start.elapsed();
            if loop_time.as_micros() > 10000 {
                log::debug!("Slow loop iteration: {}µs", loop_time.as_micros());
            }
        }
    }
}


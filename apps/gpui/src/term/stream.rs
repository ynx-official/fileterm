use anyhow::{Context, Result};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{broadcast, mpsc},
};
use tokio_serial::{DataBits, FlowControl, Parity, SerialPortBuilderExt, StopBits};

use super::{TermChunk, TerminalTransport};

const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;
const OPT_BINARY: u8 = 0;
const OPT_ECHO: u8 = 1;
const OPT_SUPPRESS_GO_AHEAD: u8 = 3;
const OPT_TERMINAL_TYPE: u8 = 24;
const OPT_NAWS: u8 = 31;

#[derive(Debug)]
enum StreamCommand {
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Shutdown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StreamProtocol {
    Raw,
    Telnet,
}

pub struct SerialConfig {
    pub device_path: String,
    pub baud_rate: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: String,
    pub flow_control: String,
}

pub struct StreamController {
    tx: broadcast::Sender<TermChunk>,
    command_tx: mpsc::UnboundedSender<StreamCommand>,
    _task: tokio::task::JoinHandle<()>,
}

impl StreamController {
    pub async fn connect_telnet(host: &str, port: u16) -> Result<Self> {
        let stream = tokio::net::TcpStream::connect((host, port))
            .await
            .with_context(|| format!("Telnet connect failed: {host}:{port}"))?;
        stream
            .set_nodelay(true)
            .context("configure Telnet TCP_NODELAY")?;
        Ok(Self::spawn(stream, StreamProtocol::Telnet))
    }

    pub fn connect_serial(config: SerialConfig) -> Result<Self> {
        let stream = tokio_serial::new(&config.device_path, config.baud_rate)
            .data_bits(match config.data_bits {
                5 => DataBits::Five,
                6 => DataBits::Six,
                7 => DataBits::Seven,
                8 => DataBits::Eight,
                _ => anyhow::bail!("Serial data bits must be 5, 6, 7, or 8"),
            })
            .stop_bits(match config.stop_bits {
                1 => StopBits::One,
                2 => StopBits::Two,
                _ => anyhow::bail!("Serial stop bits must be 1 or 2"),
            })
            .parity(match config.parity.as_str() {
                "none" => Parity::None,
                "odd" => Parity::Odd,
                "even" => Parity::Even,
                _ => anyhow::bail!("Serial parity must be none, odd, or even"),
            })
            .flow_control(match config.flow_control.as_str() {
                "none" => FlowControl::None,
                "software" => FlowControl::Software,
                "hardware" => FlowControl::Hardware,
                _ => anyhow::bail!("Serial flow control must be none, software, or hardware"),
            })
            .open_native_async()
            .with_context(|| format!("open serial device {}", config.device_path))?;
        Ok(Self::spawn(stream, StreamProtocol::Raw))
    }

    fn spawn<T>(stream: T, protocol: StreamProtocol) -> Self
    where
        T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (tx, _) = broadcast::channel(256);
        let (command_tx, mut command_rx) = mpsc::unbounded_channel();
        let output_tx = tx.clone();
        let task = tokio::spawn(async move {
            let (mut reader, mut writer) = tokio::io::split(stream);
            let mut buffer = [0_u8; 32 * 1024];
            let mut seq = 0_u64;
            let mut telnet = TelnetDecoder::default();
            loop {
                tokio::select! {
                    read = reader.read(&mut buffer) => {
                        let count = match read {
                            Ok(0) | Err(_) => break,
                            Ok(count) => count,
                        };
                        let (bytes, response) = match protocol {
                            StreamProtocol::Raw => (buffer[..count].to_vec(), Vec::new()),
                            StreamProtocol::Telnet => telnet.feed(&buffer[..count]),
                        };
                        if !response.is_empty() && writer.write_all(&response).await.is_err() {
                            break;
                        }
                        if !bytes.is_empty() {
                            seq = seq.wrapping_add(1);
                            let _ = output_tx.send(TermChunk { seq, bytes });
                        }
                    }
                    command = command_rx.recv() => {
                        match command {
                            Some(StreamCommand::Input(bytes)) => {
                                let bytes = if protocol == StreamProtocol::Telnet {
                                    escape_telnet_input(&bytes)
                                } else {
                                    bytes
                                };
                                if writer.write_all(&bytes).await.is_err() || writer.flush().await.is_err() {
                                    break;
                                }
                            }
                            Some(StreamCommand::Resize { cols, rows }) if protocol == StreamProtocol::Telnet => {
                                let frame = naws_frame(cols, rows);
                                if writer.write_all(&frame).await.is_err() {
                                    break;
                                }
                            }
                            Some(StreamCommand::Resize { .. }) => {}
                            Some(StreamCommand::Shutdown) | None => {
                                let _ = writer.shutdown().await;
                                break;
                            }
                        }
                    }
                }
            }
        });
        Self {
            tx,
            command_tx,
            _task: task,
        }
    }

    pub fn shutdown(&self) {
        let _ = self.command_tx.send(StreamCommand::Shutdown);
    }
}

impl TerminalTransport for StreamController {
    fn subscribe(&self) -> broadcast::Receiver<TermChunk> {
        self.tx.subscribe()
    }

    fn write_input(&self, bytes: &[u8]) -> Result<()> {
        self.command_tx
            .send(StreamCommand::Input(bytes.to_vec()))
            .context("terminal stream closed")
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.command_tx
            .send(StreamCommand::Resize { cols, rows })
            .context("terminal stream closed")
    }
}

impl Drop for StreamController {
    fn drop(&mut self) {
        let _ = self.command_tx.send(StreamCommand::Shutdown);
    }
}

#[derive(Default)]
struct TelnetDecoder {
    state: TelnetState,
    subnegotiation: Vec<u8>,
}

#[derive(Default)]
enum TelnetState {
    #[default]
    Data,
    Iac,
    Negotiation(u8),
    Subnegotiation,
    SubnegotiationIac,
}

impl TelnetDecoder {
    fn feed(&mut self, input: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let mut output = Vec::with_capacity(input.len());
        let mut response = Vec::new();
        for &byte in input {
            match self.state {
                TelnetState::Data if byte == IAC => self.state = TelnetState::Iac,
                TelnetState::Data => output.push(byte),
                TelnetState::Iac if byte == IAC => {
                    output.push(IAC);
                    self.state = TelnetState::Data;
                }
                TelnetState::Iac if matches!(byte, DO | DONT | WILL | WONT) => {
                    self.state = TelnetState::Negotiation(byte);
                }
                TelnetState::Iac if byte == SB => {
                    self.subnegotiation.clear();
                    self.state = TelnetState::Subnegotiation;
                }
                TelnetState::Iac => self.state = TelnetState::Data,
                TelnetState::Negotiation(command) => {
                    response.extend(negotiate(command, byte));
                    self.state = TelnetState::Data;
                }
                TelnetState::Subnegotiation if byte == IAC => {
                    self.state = TelnetState::SubnegotiationIac;
                }
                TelnetState::Subnegotiation => self.subnegotiation.push(byte),
                TelnetState::SubnegotiationIac if byte == SE => {
                    if self.subnegotiation.as_slice() == [OPT_TERMINAL_TYPE, 1] {
                        response.extend([IAC, SB, OPT_TERMINAL_TYPE, 0]);
                        response.extend_from_slice(b"xterm-256color");
                        response.extend([IAC, SE]);
                    }
                    self.subnegotiation.clear();
                    self.state = TelnetState::Data;
                }
                TelnetState::SubnegotiationIac if byte == IAC => {
                    self.subnegotiation.push(IAC);
                    self.state = TelnetState::Subnegotiation;
                }
                TelnetState::SubnegotiationIac => self.state = TelnetState::Subnegotiation,
            }
        }
        (output, response)
    }
}

fn negotiate(command: u8, option: u8) -> [u8; 3] {
    let response = match command {
        WILL if matches!(option, OPT_BINARY | OPT_ECHO | OPT_SUPPRESS_GO_AHEAD) => DO,
        WILL => DONT,
        DO if matches!(
            option,
            OPT_BINARY | OPT_SUPPRESS_GO_AHEAD | OPT_TERMINAL_TYPE | OPT_NAWS
        ) =>
        {
            WILL
        }
        DO => WONT,
        DONT => WONT,
        WONT => DONT,
        _ => DONT,
    };
    [IAC, response, option]
}

fn escape_telnet_input(input: &[u8]) -> Vec<u8> {
    let mut escaped = Vec::with_capacity(input.len());
    for &byte in input {
        escaped.push(byte);
        if byte == IAC {
            escaped.push(IAC);
        }
    }
    escaped
}

fn naws_frame(cols: u16, rows: u16) -> Vec<u8> {
    let mut frame = vec![IAC, SB, OPT_NAWS];
    for byte in [cols.to_be_bytes(), rows.to_be_bytes()]
        .into_iter()
        .flatten()
    {
        frame.push(byte);
        if byte == IAC {
            frame.push(IAC);
        }
    }
    frame.extend([IAC, SE]);
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telnet_decoder_strips_negotiation_and_replies() {
        let mut decoder = TelnetDecoder::default();
        let (output, response) = decoder.feed(&[b'h', b'i', IAC, WILL, OPT_ECHO, b'!']);
        assert_eq!(output, b"hi!");
        assert_eq!(response, [IAC, DO, OPT_ECHO]);
    }

    #[test]
    fn telnet_decoder_handles_split_iac_and_terminal_type() {
        let mut decoder = TelnetDecoder::default();
        assert_eq!(
            decoder.feed(&[IAC, SB, OPT_TERMINAL_TYPE]).0,
            Vec::<u8>::new()
        );
        let (_, response) = decoder.feed(&[1, IAC, SE]);
        assert!(response.windows(14).any(|part| part == b"xterm-256color"));
    }

    #[test]
    fn telnet_input_and_naws_escape_iac_bytes() {
        assert_eq!(escape_telnet_input(&[1, IAC, 2]), [1, IAC, IAC, 2]);
        let frame = naws_frame(255, 24);
        assert_eq!(frame, [IAC, SB, OPT_NAWS, 0, IAC, IAC, 0, 24, IAC, SE]);
    }
}

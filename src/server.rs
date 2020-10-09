//! This module defines a wrapper around Minecraft's
//! [ServerListPing](https://wiki.vg/Server_List_Ping)

use anyhow::{Context, Result};
use serde::Deserialize;
use thiserror::Error;
use tokio::net::TcpStream;

use crate::protocol::{self, AsyncReadRawPacket, AsyncWriteRawPacket};

#[derive(Error, Debug)]
pub enum ServerError {
    #[error("error reading or writing data")]
    ProtocolError,

    #[error("failed to connect to server")]
    FailedToConnect,

    #[error("invalid JSON response: \"{0}\"")]
    InvalidJson(serde_json::Error),
}

impl From<protocol::ProtocolError> for ServerError {
    fn from(_err: protocol::ProtocolError) -> Self {
        ServerError::ProtocolError
    }
}

/// Contains information about the server version.
#[derive(Debug, Deserialize)]
pub struct ServerVersion {
    /// The server's Minecraft version, i.e. "1.15.2".
    pub name: String,

    /// The server's ServerListPing protocol version.
    pub protocol: u32,
}

/// Contains information about a player.
#[derive(Debug, Deserialize)]
pub struct ServerPlayer {
    /// The player's in-game name.
    pub name: String,

    /// The player's UUID.
    pub id: String,
}

/// Contains information about the currently online
/// players.
#[derive(Debug, Deserialize)]
pub struct ServerPlayers {
    /// The configured maximum number of players for the
    /// server.
    pub max: u32,

    /// The number of players currently online.
    pub online: u32,

    /// An optional list of player information for
    /// currently online players.
    pub sample: Option<Vec<ServerPlayer>>,
}

/// Contains the server's MOTD.
#[derive(Debug, Deserialize)]
pub struct BigServerDescription {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub extra: Vec<ExtraDescriptionPart>,
}

/// Contains a segment of the extra part of a server description
#[derive(Debug, Deserialize)]
pub struct ExtraDescriptionPart {
    pub text: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
}

// TODO maybe add some more mod lists? allthough i'm not aware of other servers sending mod lists.
/// this is a response containing information about mods which modded servers send
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ModInfo {
    #[serde(rename = "FML")]
    Forge {
        #[serde(rename = "modList")]
        mod_list: Vec<ForgeModInfo>,
    },
}

/// this struct represents a modInfo entry as sent by forge
#[derive(Debug, Deserialize)]
pub struct ForgeModInfo {
    pub modid: String,
    pub version: String,
}

/// there are 2 variants of server descriptions
/// the Simple variation is rarely used, but the minecraft client understands it
/// so we should be compatible too
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ServerDescription {
    /// this is used if the `description` field in the JSON response is a String
    Simple(String),
    /// this is used if the `description` field in the JSON respons is a `BigServerDescription`
    Big(BigServerDescription),
}

impl ServerDescription {
    /// gets the text of this `ServerDescription` no matter if it is a
    /// `Simple` or `Big` description
    pub fn get_text(&self) -> &String {
        match self {
            Self::Big(desc) => &desc.text,
            Self::Simple(desc) => desc,
        }
    }
}

/// The decoded JSON response from a status query over
/// ServerListPing.
#[derive(Debug, Deserialize)]
pub struct StatusResponse {
    /// Information about the server's version.
    pub version: ServerVersion,

    /// Information about currently online players.
    pub players: ServerPlayers,

    /// Single-field struct containing the server's MOTD.
    pub description: ServerDescription,

    /// Optional field containing a path to the server's
    /// favicon.
    pub favicon: Option<String>,

    pub modinfo: Option<ModInfo>,
}

const LATEST_PROTOCOL_VERSION: usize = 578;
const DEFAULT_PORT: u16 = 25565;

/// Builder for a Minecraft
/// ServerListPing connection.
pub struct ConnectionConfig {
    protocol_version: usize,
    address: String,
    port: u16,
}

impl ConnectionConfig {
    /// Initiates the Minecraft server
    /// connection build process.
    pub fn build(address: String) -> Self {
        ConnectionConfig {
            protocol_version: LATEST_PROTOCOL_VERSION,
            address,
            port: DEFAULT_PORT,
        }
    }

    /// Sets a specific
    /// protocol version for the connection to
    /// use. If not specified, the latest version
    /// will be used.
    pub fn with_protocol_version(mut self, protocol_version: usize) -> Self {
        self.protocol_version = protocol_version;
        self
    }

    /// Sets a specific port for the
    /// connection to use. If not specified, the
    /// default port of 25565 will be used.
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Connects to the server and consumes the builder.
    pub async fn connect(self) -> Result<StatusConnection> {
        let stream = TcpStream::connect(format!("{}:{}", self.address, self.port))
            .await
            .map_err(|_| ServerError::FailedToConnect)?;

        Ok(StatusConnection {
            stream,
            protocol_version: self.protocol_version,
            address: self.address,
            port: self.port,
        })
    }
}

/// Convenience wrapper for easily connecting
/// to a server on the default port with
/// the latest protocol version.
pub async fn connect(address: String) -> Result<StatusConnection> {
    ConnectionConfig::build(address).connect().await
}

/// Wraps a built connection
pub struct StatusConnection {
    stream: TcpStream,
    protocol_version: usize,
    address: String,
    port: u16,
}

impl StatusConnection {
    /// Sends and reads the packets for the
    /// ServerListPing status call.
    pub async fn status_raw(&mut self) -> Result<String> {
        let handshake = protocol::HandshakePacket::new(
            self.protocol_version,
            self.address.to_string(),
            self.port,
        );

        self.stream
            .write_packet(handshake)
            .await
            .context("failed to write handshake packet")?;

        self.stream
            .write_packet(protocol::RequestPacket::new())
            .await
            .context("failed to write request packet")?;

        let response: protocol::ResponsePacket = self
            .stream
            .read_packet()
            .await
            .context("failed to read response packet")?;

        Ok(response.body)
    }

    pub async fn status(&mut self) -> Result<StatusResponse> {
        Ok(serde_json::from_str(&self.status_raw().await?).map_err(|e| ServerError::InvalidJson(e))?)
    }
}

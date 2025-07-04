use crate::ca::client::add_http_to_url;
use crate::p2p::client::P2PClient;
use crate::p2p::{Action, RequestData, ResponseData, ALPN_QUIC_HTTP};
use crate::util::{get_peer_id_from_identifier, get_utc_timestamp};
use crate::{Node, RepoInfo};
use anyhow::{anyhow, bail};
use callisto::{relay_node, relay_repo_info};
use dashmap::DashMap;
use futures_util::future::ok;
use jupiter::storage::Storage;
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use quinn::rustls::server::WebPkiClientVerifier;
use quinn::{
    rustls, ClientConfig, Connection, Endpoint, IdleTimeout, SendStream, ServerConfig,
    TransportConfig, VarInt,
};
use rcgen::{CertificateParams, KeyPair};
use std::borrow::Cow;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::join;
use tokio::sync::oneshot::Sender;
use tracing::{error, info};
use uuid::Uuid;
use vault::integration::VaultCore;

type ReqSenderType = Sender<Vec<u8>>;
#[derive(Clone)]
pub struct P2PNode {
    pub storage: Storage,
    pub vault: VaultCore,
    pub bootstrap_node: String,
    pub peer_id: String,
    pub host: String,
    pub port: u16,
    pub http_client: reqwest::Client,
}

impl P2PNode {
    pub fn new(
        storage: Storage,
        vault: VaultCore,
        bootstrap_node: String,
        host: String,
        port: u16,
    ) -> Self {
        let peer_id = vault.load_nostr_peerid();
        P2PNode {
            storage,
            vault,
            bootstrap_node,
            peer_id,
            host,
            port,
            http_client: Default::default(),
        }
    }

    pub fn get_peer_id(&self) -> String {
        self.peer_id.clone()
    }

    pub fn get_bootstrap_node(&self) -> String {
        self.bootstrap_node.clone()
    }

    pub fn wrapped_clone(&self) -> Arc<Self> {
        Arc::new(self.clone())
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        let addr = format!("{}:{}", self.host, self.port);
        let server_addr = SocketAddr::from_str(addr.as_str())?;
        let endpoint = self.new_server_endpoint(server_addr).await?;

        //run as p2p server
        let handler = self.wrapped_clone();
        let server_endpoint = endpoint.clone();
        let server = tokio::spawn(async move {
            while let Some(conn) = server_endpoint.accept().await {
                {
                    info!("Accepting connection");
                    let handler = handler.wrapped_clone();
                    tokio::spawn(async move {
                        if let Err(e) = handler.server_handle_connection(conn).await {
                            error!("Connection failed: {reason}", reason = e.to_string());
                        }
                    });
                }
            }
        });

        // run as client to ping relay
        let handler = self.wrapped_clone();
        let client = tokio::spawn(async move {
            if let Err(e) = handler.run_as_client(endpoint).await {
                error!("P2P: Run as client failed, {}", e);
            }
        });
        join!(server, client);
        Ok(())
    }

    async fn run_as_client(&self, endpoint: Endpoint) -> anyhow::Result<()> {
        let server_addr: SocketAddr = self.bootstrap_node.parse()?;
        let connection = endpoint
            .clone()
            .connect(server_addr, "relay")?
            .await
            .map_err(|e| anyhow!("failed to connect: {}", e))?;

        let local_peer_id = self.get_peer_id();
        tokio::spawn(async move {
            if let Err(e) = P2PNode::run_ping_task(local_peer_id, connection).await {
                error!("P2P: Ping failed, {}", e);
            };
        });
        Ok(())
    }

    async fn run_ping_task(local_peer_id: String, connection: Connection) -> anyhow::Result<()> {
        loop {
            let (mut quic_send, _) = connection.open_bi().await?;

            let ping = RequestData {
                from: local_peer_id.clone(),
                data: vec![],
                func: "".to_string(),
                action: Action::Ping,
                to: "relay".to_string(),
                req_id: Uuid::new_v4().into(),
            };
            let json = serde_json::to_string(&ping)?;
            quic_send.write_all(json.as_ref()).await?;
            quic_send.finish()?;
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    }

    async fn server_handle_connection(&self, conn: quinn::Incoming) -> anyhow::Result<()> {
        let connection = conn.await?;
        let remote_address = connection.remote_address();
        let local_ip = connection.local_ip().unwrap();
        let stable_id = connection.stable_id();
        info!("Established connection: {remote_address:#?},{local_ip:#?},{stable_id:#?}");
        let stream = connection.accept_bi().await;
        let (sender, mut recv) = match stream {
            Err(quinn::ConnectionError::ApplicationClosed { .. }) => {
                info!("connection closed");
                return Ok(());
            }
            Err(e) => {
                info!("connection error:{}", e);
                return Err(e.into());
            }
            Ok(s) => s,
        };

        //Deal data
        let buffer_vec = recv.read_to_end(1024 * 1024).await?;
        if buffer_vec.is_empty() {
            bail!("QUIC Received is empty");
        }
        let data: RequestData = match serde_json::from_slice(&buffer_vec) {
            Ok(data) => data,
            Err(e) => {
                bail!("QUIC Received Error:{:?}", e);
            }
        };
        info!(
            "QUIC Received Message from[{}], Action[{}]",
            data.from, data.action
        );

        match data.action {
            Action::Ping => self.handle_ping(data, sender).await?,
            Action::Send => {}
            Action::Call => {}
            Action::Callback => {}
            Action::RepoShare => self.handle_repo_share(data, sender).await?,
            Action::Nostr => {}
            Action::Peers => self.handle_get_peers(data, sender).await?,
            Action::Repos => self.handle_get_repos(data, sender).await?,
        }
        Ok(())
    }

    async fn handle_ping(&self, data: RequestData, sender: SendStream) -> anyhow::Result<()> {
        let storage = self.storage.relay_storage();

        let node = relay_node::Model {
            peer_id: data.from.clone(),
            r#type: "node".to_string(),
            online: true,
            last_online_time: get_utc_timestamp(),
        };
        match storage.insert_or_update_node(node).await {
            Ok(_) => {
                self.send_back(data, "ok".as_bytes().to_vec(), sender)
                    .await?
            }
            Err(_) => {
                self.send_back_err(data, "Ping with error".to_string(), sender)
                    .await?
            }
        }
        Ok(())
    }

    async fn handle_repo_share(&self, data: RequestData, sender: SendStream) -> anyhow::Result<()> {
        let repo_info: RepoInfo = serde_json::from_slice(data.data.as_slice())?;
        let repo_info_model: relay_repo_info::Model = repo_info.clone().into();
        let storage = self.storage.relay_storage().clone();
        match storage.insert_or_update_repo_info(repo_info_model).await {
            Ok(_) => {
                self.send_back(data, repo_info.identifier.into_bytes(), sender)
                    .await?
            }
            Err(_) => {
                self.send_back_err(data, "Repo share failed".to_string(), sender)
                    .await?
            }
        }
        Ok(())
    }

    async fn handle_get_peers(&self, data: RequestData, sender: SendStream) -> anyhow::Result<()> {
        let storage = self.storage.relay_storage().clone();
        match storage.get_all_node().await {
            Ok(peers) => {
                let peers: Vec<Node> = peers.iter().map(|p| p.clone().into()).collect();
                let res = serde_json::to_string(&peers)?;
                self.send_back(data, res.into_bytes(), sender).await?
            }
            Err(_) => {
                self.send_back_err(data, "Get peers failed".to_string(), sender)
                    .await?
            }
        };
        Ok(())
    }

    async fn handle_get_repos(&self, data: RequestData, sender: SendStream) -> anyhow::Result<()> {
        let storage = self.storage.relay_storage().clone();
        match storage.get_all_repo_info().await {
            Ok(repo_list) => {
                let mut repo_list: Vec<RepoInfo> =
                    repo_list.iter().map(|p| p.clone().into()).collect();
                for r in repo_list.iter_mut() {
                    if let Ok(peer_id) = get_peer_id_from_identifier(r.identifier.clone()) {
                        let node = storage.get_node_by_id(peer_id.as_str()).await.unwrap();
                        if let Some(node) = node {
                            r.peer_online = node.online;
                        }
                    }
                }
                let res = serde_json::to_string(&repo_list.clone())?;
                self.send_back(data, res.into_bytes(), sender).await?
            }
            Err(_) => {
                self.send_back_err(data, "Get repos failed".to_string(), sender)
                    .await?
            }
        };
        Ok(())
    }

    async fn send_back(
        &self,
        request_data: RequestData,
        data: Vec<u8>,
        mut sender: SendStream,
    ) -> anyhow::Result<()> {
        let response = ResponseData {
            from: request_data.to.to_string(),
            data,
            func: request_data.func.clone(),
            err: "".to_string(),
            to: request_data.from.to_string(),
            req_id: request_data.req_id.clone(),
        };

        let json = serde_json::to_string(&response)?;
        sender.write_all(json.as_bytes()).await?;
        sender.finish()?;
        Ok(())
    }

    async fn send_back_err(
        &self,
        request_data: RequestData,
        err: String,
        mut sender: SendStream,
    ) -> anyhow::Result<()> {
        let response = ResponseData {
            from: request_data.to.to_string(),
            data: vec![],
            func: request_data.func.clone(),
            err,
            to: request_data.from.to_string(),
            req_id: request_data.req_id.clone(),
        };

        let json = serde_json::to_string(&response)?;
        sender.write_all(json.as_bytes()).await?;
        sender.finish()?;
        Ok(())
    }

    async fn new_server_endpoint(&self, server_addr: SocketAddr) -> anyhow::Result<Endpoint> {
        let (user_cert, user_key) = self
            .get_user_cert_from_ca(self.get_bootstrap_node())
            .await?;
        let ca_cert = self.get_ca_cert_from_ca(self.get_bootstrap_node()).await?;

        let mut roots = rustls::RootCertStore::empty();

        roots.add(ca_cert)?;

        let client_verifier = WebPkiClientVerifier::builder(roots.into())
            .build()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let mut server_crypto = rustls::ServerConfig::builder()
            .with_client_cert_verifier(client_verifier)
            .with_single_cert(vec![user_cert], user_key)?;
        server_crypto.alpn_protocols = ALPN_QUIC_HTTP.iter().map(|&x| x.into()).collect();
        server_crypto.max_early_data_size = u32::MAX;

        let mut server_config =
            ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(server_crypto)?));

        let mut transport_config = TransportConfig::default();
        transport_config.max_idle_timeout(Some(IdleTimeout::from(VarInt::from_u32(300_000))));
        transport_config.keep_alive_interval(Some(Duration::from_secs(15)));
        server_config.transport_config(transport_config.into());

        let endpoint = Endpoint::server(server_config, server_addr)?;
        info!("Quic server listening on udp {}", endpoint.local_addr()?);
        Ok(endpoint)
    }
}

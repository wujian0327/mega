use std::{
    collections::HashMap,
    thread::{self, sleep},
    time::{self, Duration},
};

use axum::async_trait;
use common::config::{Config, ZTMConfig};
use reqwest::{header::CONTENT_TYPE, Client};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use vault::vault::{read_secret, write_secret};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ZTMUserPermit {
    pub ca: String,
    pub agent: CertAgent,
    pub bootstraps: Vec<String>,
}

impl ZTMUserPermit {
    pub fn to_json_map(&self) -> serde_json::Map<String, Value> {
        let value = serde_json::to_value(&self.clone()).unwrap();
        serde_json::from_str(&value.to_string()).unwrap()
    }

    pub fn from_json_map(map: serde_json::Map<String, Value>) -> ZTMUserPermit {
        let permit: ZTMUserPermit = serde_json::from_value(Value::Object(map)).unwrap();
        permit
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct CertAgent {
    pub name: String,
    pub certificate: String,
    #[serde(rename = "privateKey")]
    pub private_key: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ZTMMesh {
    pub name: String,
    pub ca: String,
    pub agent: Agent,
    pub bootstraps: Vec<String>,
    pub connected: bool,
    pub errors: Vec<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub username: String,
    pub certificate: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ZTMEndPoint {
    pub id: String,
    pub name: String,
    pub username: String,
    pub online: bool,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ZTMServiceReq {
    pub host: String,
    pub port: u16,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ZTMPortReq {
    pub target: ZTMPortService,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ZTMPortService {
    pub service: String,
}

const MESH_NAME: &str = "relay_mesh";

#[async_trait]
pub trait ZTMCA {
    async fn create_ztm_certificate(&self, name: String) -> Result<ZTMUserPermit, String>;
    async fn delete_ztm_certificate(&self, name: String) -> Result<String, String>;
}

#[async_trait]
pub trait ZTMAgent {
    async fn connect_ztm_hub(&self, permit: ZTMUserPermit) -> Result<ZTMMesh, String>;
    async fn create_ztm_service(
        &self,
        ep_id: String,
        service_name: String,
        port: u16,
    ) -> Result<String, String>;
    async fn create_ztm_port(
        &self,
        ep_id: String,
        service_name: String,
        port: u16,
    ) -> Result<String, String>;
}

pub struct RemoteZTM {
    pub config: ZTMConfig,
}

#[async_trait]
impl ZTMCA for RemoteZTM {
    async fn create_ztm_certificate(&self, name: String) -> Result<ZTMUserPermit, String> {
        let ca_address = self.config.ca.clone();
        let hub_address = self.config.hub.clone();

        //1. GET {ca}/api/certificates/ca -> ca certificate
        let url = format!("{ca_address}/api/certificates/ca");
        let request_result = reqwest::get(url).await;
        let ca_certificate = match handle_ztm_response(request_result).await {
            Ok(s) => s,
            Err(s) => {
                return Err(s);
            }
        };

        //2. POST {ca}/api/certificates/{username} -> user private key
        let url = format!("{ca_address}/api/certificates/{name}");
        let client = Client::new();
        let request_result = client.post(url).send().await;
        let user_key = match handle_ztm_response(request_result).await {
            Ok(s) => s,
            Err(s) => {
                return Err(s);
            }
        };

        //3. GET {ca}/api/certificates/{username} -> user certificate
        let url = format!("{ca_address}/api/certificates/{name}");
        let request_result = reqwest::get(url).await;
        let user_certificate = match handle_ztm_response(request_result).await {
            Ok(s) => s,
            Err(s) => {
                return Err(s);
            }
        };

        // Combine those into a json permit
        let agent = CertAgent {
            name: name.clone(),
            certificate: user_certificate.clone(),
            private_key: user_key.clone(),
        };

        let hub_address = hub_address.replace("http://", "");
        let permit = ZTMUserPermit {
            ca: ca_certificate.clone(),
            agent,
            bootstraps: vec![hub_address],
        };

        let permit_json = serde_json::to_string(&permit).unwrap();
        tracing::info!("new permit [{name}]: {permit_json}");

        Ok(permit)
    }

    async fn delete_ztm_certificate(&self, name: String) -> Result<String, String> {
        let ca_address = self.config.ca.clone();

        //1. DELETE /api/certificates/${username}
        let url = format!("{ca_address}/api/certificates/{name}");
        let client = Client::new();
        let request_result = client.delete(url).send().await;
        let s: String = match handle_ztm_response(request_result).await {
            Ok(s) => s,
            Err(s) => {
                return Err(s);
            }
        };
        Ok(s)
    }
}

#[async_trait]
impl ZTMAgent for RemoteZTM {
    async fn connect_ztm_hub(&self, permit: ZTMUserPermit) -> Result<ZTMMesh, String> {
        // POST {agent}/api/meshes/${meshName}
        let permit_string = serde_json::to_string(&permit).unwrap();
        let agent_address = self.config.agent.clone();
        let url = format!("{agent_address}/api/meshes/{MESH_NAME}");
        let client = Client::new();
        let request_result = client
            .post(url)
            .header(CONTENT_TYPE, "application/json")
            .body(permit_string)
            .send()
            .await;
        let response_text = match handle_ztm_response(request_result).await {
            Ok(s) => s,
            Err(s) => {
                return Err(s);
            }
        };

        let mesh: ZTMMesh = match serde_json::from_slice(response_text.as_bytes()) {
            Ok(p) => p,
            Err(e) => {
                return Err(e.to_string());
            }
        };
        Ok(mesh)
    }

    async fn create_ztm_service(
        &self,
        ep_id: String,
        service_name: String,
        port: u16,
    ) -> Result<String, String> {
        //  create a ZTM service
        //  POST {agent}/api/meshes/${mesh.name}/endpoints/${ep.id}/services/${svcName}
        let agent_address = self.config.agent.clone();
        let url = format!(
            "{agent_address}/api/meshes/{MESH_NAME}/endpoints/{ep_id}/services/tcp/{service_name}"
        );
        let client = Client::new();
        let req = ZTMServiceReq {
            host: "127.0.0.1".to_string(),
            port,
        };
        let req_string = serde_json::to_string(&req).unwrap();
        let request_result = client
            .post(url)
            .header(CONTENT_TYPE, "application/json")
            .body(req_string)
            .send()
            .await;
        let response_text = match handle_ztm_response(request_result).await {
            Ok(s) => s,
            Err(s) => {
                return Err(s);
            }
        };
        Ok(response_text)
    }

    async fn create_ztm_port(
        &self,
        ep_id: String,
        service_name: String,
        port: u16,
    ) -> Result<String, String> {
        //POST {agent}/api/meshes/${mesh.name}/endpoints/${ep.id}/ports/127.0.0.1/tcp/{port}
        // request body {"service":service_name}
        let agent_address = self.config.agent.clone();
        let url = format!(
            "{agent_address}/api/meshes/{MESH_NAME}/endpoints/{ep_id}/ports/127.0.0.1/tcp/{port}"
        );
        let client = Client::new();
        let req = ZTMPortReq {
            target: ZTMPortService {
                service: service_name,
            },
        };
        let req_string = serde_json::to_string(&req).unwrap();
        let request_result = client
            .post(url)
            .header(CONTENT_TYPE, "application/json")
            .body(req_string)
            .send()
            .await;
        let response_text = match handle_ztm_response(request_result).await {
            Ok(s) => s,
            Err(s) => {
                return Err(s);
            }
        };
        Ok(response_text)
    }
}

#[derive(Debug, Clone)]
pub struct LocalZTMAgent {
    pub agent_port: u16,
}

impl LocalZTMAgent {
    pub fn start_ztm_agent(self) {
        tokio::spawn(async move {
            rust_ztm::start_agent("ztm_agent.db", self.agent_port);
        });
    }
}

#[derive(Debug, Clone)]
pub struct LocalZTMHub {
    pub hub_port: u16,
    pub ca: String,
    pub name: Vec<String>,
}

impl LocalZTMHub {
    pub fn start_ztm_hub(self) {
        tokio::spawn(async move {
            rust_ztm::start_hub(self.hub_port, self.name, &self.ca);
        });
    }
}

#[async_trait]
impl ZTMAgent for LocalZTMAgent {
    async fn connect_ztm_hub(&self, permit: ZTMUserPermit) -> Result<ZTMMesh, String> {
        // POST {agent}/api/meshes/${meshName}
        let permit_string = serde_json::to_string(&permit).unwrap();
        // tracing::info!("permit_string:{permit_string}");
        let agent_port = self.agent_port;
        let agent_address = format!("http://127.0.0.1:{agent_port}");
        let url = format!("{agent_address}/api/meshes/{MESH_NAME}");
        let client = Client::new();
        let request_result = client
            .post(url)
            .header(CONTENT_TYPE, "application/json")
            .body(permit_string)
            .send()
            .await;
        let response_text = match handle_ztm_response(request_result).await {
            Ok(s) => s,
            Err(s) => {
                return Err(s);
            }
        };

        let mesh: ZTMMesh = match serde_json::from_slice(response_text.as_bytes()) {
            Ok(p) => p,
            Err(e) => {
                return Err(e.to_string());
            }
        };
        Ok(mesh)
    }

    async fn create_ztm_service(
        &self,
        ep_id: String,
        service_name: String,
        port: u16,
    ) -> Result<String, String> {
        //  create a ZTM service
        //  POST {agent}/api/meshes/${mesh.name}/endpoints/${ep.id}/services/${svcName}
        let agent_port = self.agent_port;
        let agent_address = format!("http://127.0.0.1:{agent_port}");
        let url = format!(
            "{agent_address}/api/meshes/{MESH_NAME}/endpoints/{ep_id}/services/tcp/{service_name}"
        );
        let client = Client::new();
        let req = ZTMServiceReq {
            host: "127.0.0.1".to_string(),
            port,
        };
        let req_string = serde_json::to_string(&req).unwrap();
        let request_result = client
            .post(url)
            .header(CONTENT_TYPE, "application/json")
            .body(req_string)
            .send()
            .await;
        let response_text = match handle_ztm_response(request_result).await {
            Ok(s) => s,
            Err(s) => {
                return Err(s);
            }
        };
        Ok(response_text)
    }

    async fn create_ztm_port(
        &self,
        ep_id: String,
        service_name: String,
        port: u16,
    ) -> Result<String, String> {
        //POST {agent}/api/meshes/${mesh.name}/endpoints/${ep.id}/ports/127.0.0.1/tcp/{port}
        // request body {"service":service_name}
        let agent_port = self.agent_port;
        let agent_address = format!("http://127.0.0.1:{agent_port}");
        let url = format!(
            "{agent_address}/api/meshes/{MESH_NAME}/endpoints/{ep_id}/ports/127.0.0.1/tcp/{port}"
        );
        let client = Client::new();
        let req = ZTMPortReq {
            target: ZTMPortService {
                service: service_name,
            },
        };
        let req_string = serde_json::to_string(&req).unwrap();
        let request_result = client
            .post(url)
            .header(CONTENT_TYPE, "application/json")
            .body(req_string)
            .send()
            .await;
        let response_text = match handle_ztm_response(request_result).await {
            Ok(s) => s,
            Err(s) => {
                return Err(s);
            }
        };
        Ok(response_text)
    }
}

pub async fn run_ztm_client(
    bootstrap_node: String,
    _config: Config,
    peer_id: String,
    agent: LocalZTMAgent,
) {
    let name = peer_id.clone();
    // let _context = Context::new(config.clone()).await;
    let local_permit_option = read_secret(peer_id.as_str()).unwrap();
    let permit: ZTMUserPermit = match local_permit_option {
        Some(res) => {
            let p = ZTMUserPermit::from_json_map(res.data.unwrap());
            tracing::info!("read ztm permit file from vault:{:?}", p);
            p
        }
        None => {
            //generate permit from bootstrap_node
            // 1. to get permit json from bootstrap_node
            // GET {bootstrap_node}/api/v1/certificate?name={name}
            let url = format!("{bootstrap_node}/api/v1/certificate?name={name}");
            let request_result = reqwest::get(url).await;
            let response_text = match handle_ztm_response(request_result).await {
                Ok(s) => s,
                Err(s) => {
                    tracing::error!(
                        "GET {bootstrap_node}/api/v1/certificate?name={name} failed,{s}"
                    );
                    return;
                }
            };
            let permit: ZTMUserPermit = match serde_json::from_slice(response_text.as_bytes()) {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!("{}", e);
                    return;
                }
            };
            //save to vault
            write_secret(peer_id.clone().as_str(), Some(permit.to_json_map().clone())).unwrap();
            permit
        }
    };

    // 2. join ztm mesh
    let mesh = match agent.connect_ztm_hub(permit.clone()).await {
        Ok(m) => m,
        Err(s) => {
            tracing::error!(s);
            return;
        }
    };
    tracing::info!("connect to ztm hub successfully");

    // 3. create a ztm port for relay
    let ztm_port = 8002;
    match agent
        .create_ztm_port(mesh.agent.id, "relay".to_string(), ztm_port)
        .await
    {
        Ok(_) => (),
        Err(s) => {
            tracing::error!("create a ztm port failed, {s}");
            return;
        }
    }
    tracing::info!("create a ztm port successfully, port:{ztm_port}");
    let peer_id_clone = peer_id.clone();
    loop {
        ping(
            peer_id_clone.clone(),
            permit.bootstraps.first().unwrap().to_string(),
            ztm_port,
        )
        .await;
        sleep(Duration::from_secs(15));
    }
}

pub async fn ping(peer_id: String, hub: String, ztm_port: u16) {
    let url = format!("http://127.0.0.1:{ztm_port}/ping");
    let mut params = HashMap::new();
    params.insert("peer_id", peer_id.clone());
    params.insert("hub", hub);
    params.insert("agent_name", peer_id.clone());
    params.insert("service_name", peer_id.clone());
    let client = reqwest::Client::new();
    let response = client.get(url.clone()).query(&params).send().await;
    let response_text = match handle_ztm_response(response).await {
        Ok(s) => s,
        Err(s) => {
            tracing::error!("GET {url} failed,{s}");
            return;
        }
    };
    tracing::info!("Get {url}, response: {response_text}");
}

pub async fn handle_ztm_response(
    request_result: Result<reqwest::Response, reqwest::Error>,
) -> Result<String, String> {
    match request_result {
        Ok(res) => {
            if res.status().is_success() {
                Ok(res.text().await.unwrap())
            } else {
                Err(res.text().await.unwrap())
            }
        }
        Err(e) => Err(e.to_string()),
    }
}

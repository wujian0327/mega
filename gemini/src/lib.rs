use serde::{Deserialize, Serialize};

pub mod http;
pub mod ztm;

#[derive(Deserialize, Debug)]
pub struct RelayGetParams {
    pub peer_id: Option<String>,
    pub hub: Option<String>,
    pub name: Option<String>,
    pub agent_name: Option<String>,
    pub service_name: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RelayPingRes {
    pub success: bool,
}

#[derive(Debug)]
pub enum MegaType {
    Mega,
    Relay,
}
impl MegaType {
    pub fn to_string(&self) -> String {
        match self {
            MegaType::Mega => "Mega".to_string(),
            MegaType::Relay => "Relay".to_string(),
        }
    }
}

use chrono::Utc;
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
pub struct RelayResultRes {
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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Node {
    pub peer_id: String,
    pub hub: String,
    pub agent_name: String,
    pub service_name: String,
    pub mega_type: String,
    pub online: bool,
    pub last_online_time: i64,
}

#[derive(Debug)]
pub enum ConversionError {
    InvalidParas,
}

impl TryFrom<RelayGetParams> for Node {
    type Error = ConversionError;

    fn try_from(paras: RelayGetParams) -> Result<Self, Self::Error> {
        if paras.peer_id.is_none()
            || paras.hub.is_none()
            || paras.agent_name.is_none()
            || paras.service_name.is_none()
        {
            return Err(ConversionError::InvalidParas);
        }
        // 获取当前时间
        let now = Utc::now();
        let timestamp_milliseconds = now.timestamp_millis();
        Ok(Node {
            peer_id: paras.peer_id.unwrap(),
            hub: paras.hub.unwrap(),
            agent_name: paras.agent_name.unwrap(),
            service_name: paras.service_name.unwrap(),
            mega_type: MegaType::Mega.to_string(),
            online: true,
            last_online_time: timestamp_milliseconds,
        })
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct RepoInfo {
    pub name: String,
    pub identifier: String,
    pub origin: String,
    pub update_time: i64,
    pub commit: String,
}

use agent::{LocalZTMAgent, ZTMAgent};

use crate::{util::get_short_peer_id, ZTM_APP_PROVIDER};

pub mod agent;
pub mod hub;

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

pub async fn create_tunnel(
    ztm_agent_port: u16,
    remote_peer_id: String,
    local_port: u16,
    remote_port: u16,
) -> Result<(), String> {
    let agent: LocalZTMAgent = LocalZTMAgent {
        agent_port: ztm_agent_port,
    };
    let local_ep = match agent.get_ztm_local_endpoint().await {
        Ok(ep) => ep,
        Err(e) => return Err(e),
    };

    let remote_ep = match agent.get_ztm_remote_endpoint(remote_peer_id.clone()).await {
        Ok(ep) => ep,
        Err(e) => return Err(e),
    };

    let (peer_id, _) = vault::init();
    let bound_name = format!(
        "{}_{}",
        get_short_peer_id(peer_id),
        get_short_peer_id(remote_peer_id)
    );
    //creata inbound
    match agent
        .create_ztm_app_tunnel_inbound(
            local_ep.id,
            ZTM_APP_PROVIDER.to_string(),
            "tunnel".to_string(),
            bound_name.clone(),
            local_port,
        )
        .await
    {
        Ok(_) => (),
        Err(s) => {
            tracing::error!("create app inbound, {s}");
            return Err(s);
        }
    }
    tracing::info!("create app inbound successfully");

    //creata outbound
    match agent
        .create_ztm_app_tunnel_outbound(
            remote_ep.id,
            ZTM_APP_PROVIDER.to_string(),
            "tunnel".to_string(),
            bound_name,
            remote_port,
        )
        .await
    {
        Ok(msg) => {
            tracing::info!("create app outbound successfully,{}", msg);
        }
        Err(s) => {
            tracing::error!("create app outbound, {s}");
            return Err(s);
        }
    }
    Ok(())
}

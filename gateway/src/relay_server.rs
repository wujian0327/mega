use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Mutex;

use crate::api_service;
use crate::api_service::router::ApiServiceState;
use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{Request, Response, StatusCode, Uri};
use axum::routing::get;
use axum::Router;
use chrono::Utc;
use common::config::{Config, ZTMConfig};
use gemini::ztm::{RemoteZTM, ZTM};
use gemini::{MegaType, RelayGetParams, RelayPingRes};
use jupiter::context::Context;
use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tower::ServiceBuilder;
use tower_http::cors::{Any, CorsLayer};
use tower_http::decompression::RequestDecompressionLayer;
use tower_http::trace::TraceLayer;

pub async fn run_relay_server(config: Config, host: String, port: u16) {
    let app = app(config.clone(), host.clone(), port).await;

    let ztm_config = config.ztm;
    match relay_connect_ztm(ztm_config, port).await {
        Ok(s) => {
            tracing::info!("relay connect ztm success: {s}");
        }
        Err(e) => {
            tracing::error!("relay connect ztm failed : {e}");
            return;
        }
    }

    let server_url = format!("{}:{}", host, port);
    tracing::info!("start relay server: {server_url}");
    let addr = SocketAddr::from_str(&server_url).unwrap();
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}

#[derive(Clone)]
pub struct AppState {
    pub context: Context,
    pub host: String,
    pub port: u16,
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

lazy_static! {
    static ref NODELIST: Mutex<Vec<Node>> = Mutex::new(vec![]);
}

pub async fn app(config: Config, host: String, port: u16) -> Router {
    let state = AppState {
        host,
        port,
        context: Context::new(config.clone()).await,
    };

    let api_state = ApiServiceState {
        context: Context::new(config).await,
    };

    // add RequestDecompressionLayer for handle gzip encode
    // add TraceLayer for log record
    // add CorsLayer to add cors header
    Router::new()
        .nest(
            "/api/v1",
            api_service::router::routers().with_state(api_state),
        )
        .route(
            "/*path",
            get(get_method_router).post(post_method_router),
            // .put(put_method_router),
        )
        .layer(ServiceBuilder::new().layer(CorsLayer::new().allow_origin(Any)))
        .layer(TraceLayer::new_for_http())
        .layer(RequestDecompressionLayer::new())
        .with_state(state)
}

async fn get_method_router(
    state: State<AppState>,
    Query(params): Query<RelayGetParams>,
    uri: Uri,
) -> Result<Response<Body>, (StatusCode, String)> {
    let ztm_config = state.context.config.ztm.clone();
    if Regex::new(r"/hello$").unwrap().is_match(uri.path()) {
        return hello_relay(params).await;
    } else if Regex::new(r"/ping$").unwrap().is_match(uri.path()) {
        return ping(params).await;
    } else if Regex::new(r"/nodo_list$").unwrap().is_match(uri.path()) {
        return nodo_list(params).await;
    } else if Regex::new(r"/certificate$").unwrap().is_match(uri.path()) {
        return certificate(ztm_config, params).await;
    }
    Err((
        StatusCode::NOT_FOUND,
        String::from("Operation not supported\n"),
    ))
}

async fn post_method_router(
    state: State<AppState>,
    _uri: Uri,
    _req: Request<Body>,
) -> Result<Response<Body>, (StatusCode, String)> {
    let _ztm_config = state.context.config.ztm.clone();
    Err((
        StatusCode::NOT_FOUND,
        String::from("Operation not supported\n"),
    ))
}

pub async fn hello_relay(_params: RelayGetParams) -> Result<Response<Body>, (StatusCode, String)> {
    Ok(Response::builder().body(Body::from("hello relay")).unwrap())
}

pub async fn ping(params: RelayGetParams) -> Result<Response<Body>, (StatusCode, String)> {
    let node = match Node::try_from(params) {
        Ok(n) => n,
        Err(_) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "invalid paras".to_string(),
            ));
        }
    };
    {
        let mut nodelist = NODELIST.lock().unwrap();
        if let Some(pos) = nodelist.iter().position(|n| n.peer_id == node.peer_id) {
            nodelist.remove(pos);
        }
        nodelist.push(node);
    }
    let res = serde_json::to_string(&RelayPingRes { success: true }).unwrap();
    Ok(Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(res))
        .unwrap())
}

pub async fn nodo_list(_params: RelayGetParams) -> Result<Response<Body>, (StatusCode, String)> {
    let nodelist = NODELIST.lock().unwrap();
    let json_string = serde_json::to_string(&*nodelist).unwrap();
    Ok(Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(json_string))
        .unwrap())
}

pub async fn certificate(
    config: ZTMConfig,
    params: RelayGetParams,
) -> Result<Response<Body>, (StatusCode, String)> {
    if params.name.is_none() {
        return Err((StatusCode::BAD_REQUEST, "not enough paras".to_string()));
    }
    let name = params.name.unwrap();

    let ztm: RemoteZTM = RemoteZTM { config };
    let permit = match ztm.create_ztm_certificate(name.clone()).await {
        Ok(p) => p,
        Err(e) => {
            return Err((StatusCode::INTERNAL_SERVER_ERROR, e));
        }
    };

    let permit_json = serde_json::to_string(&permit).unwrap();
    tracing::info!("new permit [{name}]: {permit_json}");

    Ok(Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(permit_json))
        .unwrap())
}

pub async fn relay_connect_ztm(config: ZTMConfig, relay_port: u16) -> Result<String, String> {
    // 1. generate a permit for relay
    let name = "relay".to_string();
    let ztm: RemoteZTM = RemoteZTM { config };
    match ztm.delete_ztm_certificate(name.clone()).await {
        Ok(_s) => (),
        Err(e) => {
            return Err(e);
        }
    }
    let permit = match ztm.create_ztm_certificate(name).await {
        Ok(p) => p,
        Err(e) => {
            return Err(e);
        }
    };

    // 2. connect to ZTM hub (join a mesh)
    let mesh = match ztm.connect_ztm_hub(permit).await {
        Ok(m) => m,
        Err(e) => {
            return Err(e);
        }
    };

    // 3. create a ZTM service
    let response_text = match ztm
        .create_ztm_service(mesh.agent.id, "relay".to_string(), relay_port)
        .await
    {
        Ok(m) => m,
        Err(e) => {
            return Err(e);
        }
    };

    Ok(response_text)
}

#[cfg(test)]
mod tests {}

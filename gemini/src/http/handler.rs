use jupiter::context::Context;
use venus::import_repo::repo::Repo;

use crate::{
    ztm::{agent::share_repo, create_tunnel},
    RepoInfo,
};

pub async fn repo_provide(
    port: u16,
    bootstrap_node: String,
    context: Context,
    path: String,
) -> Result<String, String> {
    let url = format!("{bootstrap_node}/api/v1/repo_provide");
    let git_model = context
        .services
        .git_db_storage
        .find_git_repo_exact_match(path.as_str())
        .await;

    let git_model = match git_model {
        Ok(r) => {
            if let Some(m) = r {
                m
            } else {
                return Err(String::from("Repo not found"));
            }
        }
        Err(_) => return Err(String::from("Repo not found")),
    };
    let repo: Repo = git_model.clone().into();
    let git_ref = context
        .services
        .git_db_storage
        .get_default_ref(&repo)
        .await
        .unwrap()
        .unwrap();

    let name = git_model.repo_name;
    let repo_path = git_model.repo_path;
    let (peer_id, _) = vault::init();
    let identifier = format!("p2p://{}/{port}{repo_path}.git", peer_id.clone());
    let update_time = git_model.created_at.and_utc().timestamp();
    let repo_info = RepoInfo {
        name,
        identifier,
        origin: peer_id.clone(),
        update_time,
        commit: git_ref.ref_hash,
        peer_online: true,
    };
    share_repo(url.clone(), repo_info).await;
    Ok("success".to_string())
}

pub async fn repo_folk(
    ztm_agent_port: u16,
    identifier: String,
    local_port: u16,
) -> Result<String, String> {
    let remote_peer_id = match get_peer_id_from_identifier(identifier.clone()) {
        Ok(p) => p,
        Err(e) => return Err(e),
    };
    let remote_port = match get_remote_port_from_identifier(identifier.clone()) {
        Ok(p) => p,
        Err(e) => return Err(e),
    };
    let git_path = match get_git_path_from_identifier(identifier) {
        Ok(p) => p,
        Err(e) => return Err(e),
    };

    match create_tunnel(ztm_agent_port, remote_peer_id, local_port, remote_port).await {
        Ok(_) => (),
        Err(e) => return Err(e),
    }

    let msg = format!("git clone http://localhost:{local_port}/{git_path}");
    Ok(msg)
}

pub fn get_peer_id_from_identifier(identifier: String) -> Result<String, String> {
    // p2p://mrJ46F8gd2sa2Dx3iCYf6DauJ2WpAaepus7PwyZVebgD/8000/third-part/mega_143.git
    let words: Vec<&str> = identifier.split('/').collect();
    if words.len() <= 2 {
        return Err("invalid identifier".to_string());
    }
    return Ok(words.get(2).unwrap().to_string());
}

pub fn get_remote_port_from_identifier(identifier: String) -> Result<u16, String> {
    // p2p://mrJ46F8gd2sa2Dx3iCYf6DauJ2WpAaepus7PwyZVebgD/8000/third-part/mega_143.git
    let words: Vec<&str> = identifier.split('/').collect();
    if words.len() <= 3 {
        return Err("invalid identifier".to_string());
    }
    match words.get(3).unwrap().parse::<u16>() {
        Ok(number) => Ok(number),
        Err(e) => Err(e.to_string()),
    }
}

pub fn get_git_path_from_identifier(identifier: String) -> Result<String, String> {
    // p2p://mrJ46F8gd2sa2Dx3iCYf6DauJ2WpAaepus7PwyZVebgD/8000/third-part/mega_143.git
    let words: Vec<&str> = identifier.split('/').collect();
    if words.len() <= 4 {
        return Err("invalid identifier".to_string());
    }
    let path = words[4..].join("/");
    Ok(path)
}

#[cfg(test)]
mod tests {}

use std::sync::Arc;

use callisto::relay_node;
use sea_orm::{DatabaseConnection, EntityTrait, InsertResult, IntoActiveModel};

use common::errors::MegaError;

#[derive(Clone)]
pub struct RelayStorage {
    pub connection: Arc<DatabaseConnection>,
}

impl RelayStorage {
    pub fn get_connection(&self) -> &DatabaseConnection {
        &self.connection
    }

    pub async fn new(connection: Arc<DatabaseConnection>) -> Self {
        RelayStorage { connection }
    }

    pub fn mock() -> Self {
        RelayStorage {
            connection: Arc::new(DatabaseConnection::default()),
        }
    }

    pub async fn get_node_by_id(
        &self,
        peer_id: &str,
    ) -> Result<Option<relay_node::Model>, MegaError> {
        let result = relay_node::Entity::find_by_id(peer_id)
            .one(self.get_connection())
            .await
            .unwrap();
        Ok(result)
    }

    pub async fn new_node(
        &self,
        node: relay_node::Model,
    ) -> Result<InsertResult<relay_node::ActiveModel>, MegaError> {
        Ok(relay_node::Entity::insert(node.into_active_model())
            .exec(self.get_connection())
            .await
            .unwrap())
    }

    pub async fn update_node(
        &self,
        node: relay_node::Model,
    ) -> Result<relay_node::Model, MegaError> {
        Ok(relay_node::Entity::update(node.into_active_model())
            .exec(self.get_connection())
            .await
            .unwrap())
    }

    pub async fn delete_node_by_id(&self, id: String) {
        relay_node::Entity::delete_by_id(id)
            .exec(self.get_connection())
            .await
            .unwrap();
    }
}

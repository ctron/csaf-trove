use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "sync_runs")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub timestamp: String,
    pub documents_changed: i64,
}

impl ActiveModelBehavior for ActiveModel {}

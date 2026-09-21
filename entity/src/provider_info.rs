use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "provider_info")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: i64,
    pub canonical_url: String,
    pub publisher_name: String,
    pub publisher_category: String,
    pub publisher_namespace: String,
    pub role: Option<String>,
    #[sea_orm(default_value = 1)]
    pub list_on_aggregators: i32,
    #[sea_orm(default_value = 1)]
    pub mirror_on_aggregators: i32,
    pub last_updated: String,
}

impl ActiveModelBehavior for ActiveModel {}

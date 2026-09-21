use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "check_failures")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub document_id: i64,
    pub profile: String,
    pub test_id: String,
    pub message: String,
    #[sea_orm(default_value = "error")]
    pub severity: String,
    #[sea_orm(belongs_to, from = "document_id", to = "id")]
    pub document: BelongsTo<super::document::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "revision_history")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub document_id: i64,
    pub version: String,
    pub date: String,
    pub summary: String,
    #[sea_orm(belongs_to, from = "document_id", to = "id")]
    pub document: BelongsTo<super::document::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}

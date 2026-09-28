use sea_orm::entity::prelude::*;

/// A distinct version of a document, as recorded in the provider's git history.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "document_versions")]
pub struct Model {
    /// Insertion-ordered key; higher IDs are newer versions.
    #[sea_orm(primary_key)]
    pub id: i64,
    /// Advisory URL identifying the document across its lifetime.
    pub url: String,
    /// Commit SHA where this version was recorded.
    pub commit_id: String,
    /// Commit timestamp as Unix seconds.
    pub timestamp: i64,
    /// Commit message.
    pub message: String,
    /// Git blob OID of the document content at this version.
    pub blob_id: String,
    /// CSAF `document.tracking.status` at this version.
    pub status: Option<String>,
    /// CSAF `document.tracking.version` at this version.
    pub version: Option<String>,
    /// CSAF `document.tracking.current_release_date` at this version.
    pub current_release_date: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}

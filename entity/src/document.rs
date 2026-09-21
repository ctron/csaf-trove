use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "documents")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub tracking_id: String,
    pub title: String,
    pub url: String,
    pub basic_passed: Option<i32>,
    pub basic_error_count: Option<i64>,
    pub basic_warning_count: Option<i64>,
    pub basic_info_count: Option<i64>,
    pub extended_passed: Option<i32>,
    pub extended_error_count: Option<i64>,
    pub extended_warning_count: Option<i64>,
    pub extended_info_count: Option<i64>,
    pub full_passed: Option<i32>,
    pub full_error_count: Option<i64>,
    pub full_warning_count: Option<i64>,
    pub full_info_count: Option<i64>,
    pub signature_present: i32,
    pub signature_error: Option<String>,
    pub category: Option<String>,
    pub publisher_name: Option<String>,
    pub initial_release_date: Option<String>,
    pub current_release_date: Option<String>,
    pub status: Option<String>,
    pub revision: Option<String>,
    pub aggregate_severity: Option<String>,
    pub csaf_version: Option<String>,
    #[sea_orm(has_many)]
    pub check_failures: HasMany<super::check_failure::Entity>,
    #[sea_orm(has_many)]
    pub revision_history: HasMany<super::revision_history::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}

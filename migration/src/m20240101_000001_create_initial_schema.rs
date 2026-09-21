use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20240101_000001_create_initial_schema"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Documents::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Documents::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Documents::TrackingId).string().not_null())
                    .col(ColumnDef::new(Documents::Title).string().not_null())
                    .col(ColumnDef::new(Documents::Url).string().not_null())
                    .col(ColumnDef::new(Documents::BasicPassed).integer())
                    .col(ColumnDef::new(Documents::BasicErrorCount).big_integer())
                    .col(ColumnDef::new(Documents::BasicWarningCount).big_integer())
                    .col(ColumnDef::new(Documents::BasicInfoCount).big_integer())
                    .col(ColumnDef::new(Documents::ExtendedPassed).integer())
                    .col(ColumnDef::new(Documents::ExtendedErrorCount).big_integer())
                    .col(ColumnDef::new(Documents::ExtendedWarningCount).big_integer())
                    .col(ColumnDef::new(Documents::ExtendedInfoCount).big_integer())
                    .col(ColumnDef::new(Documents::FullPassed).integer())
                    .col(ColumnDef::new(Documents::FullErrorCount).big_integer())
                    .col(ColumnDef::new(Documents::FullWarningCount).big_integer())
                    .col(ColumnDef::new(Documents::FullInfoCount).big_integer())
                    .col(
                        ColumnDef::new(Documents::SignaturePresent)
                            .integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Documents::SignatureError).string())
                    .col(ColumnDef::new(Documents::Category).string())
                    .col(ColumnDef::new(Documents::PublisherName).string())
                    .col(ColumnDef::new(Documents::InitialReleaseDate).string())
                    .col(ColumnDef::new(Documents::CurrentReleaseDate).string())
                    .col(ColumnDef::new(Documents::Status).string())
                    .col(ColumnDef::new(Documents::Revision).string())
                    .col(ColumnDef::new(Documents::AggregateSeverity).string())
                    .col(ColumnDef::new(Documents::CsafVersion).string())
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(CheckFailures::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(CheckFailures::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(CheckFailures::DocumentId)
                            .big_integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(CheckFailures::Profile).string().not_null())
                    .col(ColumnDef::new(CheckFailures::TestId).string().not_null())
                    .col(ColumnDef::new(CheckFailures::Message).string().not_null())
                    .col(
                        ColumnDef::new(CheckFailures::Severity)
                            .string()
                            .not_null()
                            .default("error"),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(CheckFailures::Table, CheckFailures::DocumentId)
                            .to(Documents::Table, Documents::Id),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(RevisionHistory::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(RevisionHistory::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(RevisionHistory::DocumentId)
                            .big_integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(RevisionHistory::Version).string().not_null())
                    .col(ColumnDef::new(RevisionHistory::Date).string().not_null())
                    .col(ColumnDef::new(RevisionHistory::Summary).string().not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .from(RevisionHistory::Table, RevisionHistory::DocumentId)
                            .to(Documents::Table, Documents::Id),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(SyncRuns::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(SyncRuns::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(SyncRuns::Timestamp).string().not_null())
                    .col(
                        ColumnDef::new(SyncRuns::DocumentsChanged)
                            .big_integer()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(ProviderInfoTable::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ProviderInfoTable::Id)
                            .big_integer()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ProviderInfoTable::CanonicalUrl)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProviderInfoTable::PublisherName)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProviderInfoTable::PublisherCategory)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProviderInfoTable::PublisherNamespace)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(ProviderInfoTable::Role).string())
                    .col(
                        ColumnDef::new(ProviderInfoTable::ListOnAggregators)
                            .integer()
                            .not_null()
                            .default(1),
                    )
                    .col(
                        ColumnDef::new(ProviderInfoTable::MirrorOnAggregators)
                            .integer()
                            .not_null()
                            .default(1),
                    )
                    .col(
                        ColumnDef::new(ProviderInfoTable::LastUpdated)
                            .string()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_documents_tracking_id")
                    .table(Documents::Table)
                    .col(Documents::TrackingId)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_check_failures_document_id")
                    .table(CheckFailures::Table)
                    .col(CheckFailures::DocumentId)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_revision_history_document_id")
                    .table(RevisionHistory::Table)
                    .col(RevisionHistory::DocumentId)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(CheckFailures::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(RevisionHistory::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(SyncRuns::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(ProviderInfoTable::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Documents::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Documents {
    Table,
    Id,
    TrackingId,
    Title,
    Url,
    BasicPassed,
    BasicErrorCount,
    BasicWarningCount,
    BasicInfoCount,
    ExtendedPassed,
    ExtendedErrorCount,
    ExtendedWarningCount,
    ExtendedInfoCount,
    FullPassed,
    FullErrorCount,
    FullWarningCount,
    FullInfoCount,
    SignaturePresent,
    SignatureError,
    Category,
    PublisherName,
    InitialReleaseDate,
    CurrentReleaseDate,
    Status,
    Revision,
    AggregateSeverity,
    CsafVersion,
}

#[derive(DeriveIden)]
enum CheckFailures {
    Table,
    Id,
    DocumentId,
    Profile,
    TestId,
    Message,
    Severity,
}

#[derive(DeriveIden)]
enum RevisionHistory {
    Table,
    Id,
    DocumentId,
    Version,
    Date,
    Summary,
}

#[derive(DeriveIden)]
enum SyncRuns {
    Table,
    Id,
    Timestamp,
    DocumentsChanged,
}

#[derive(DeriveIden)]
enum ProviderInfoTable {
    Table,
    Id,
    CanonicalUrl,
    PublisherName,
    PublisherCategory,
    PublisherNamespace,
    Role,
    ListOnAggregators,
    MirrorOnAggregators,
    LastUpdated,
}

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Documents::Table)
                    .add_column(ColumnDef::new(Documents::BasicTestCount).big_integer())
                    .add_column(ColumnDef::new(Documents::BasicFailingTestCount).big_integer())
                    .add_column(ColumnDef::new(Documents::ExtendedTestCount).big_integer())
                    .add_column(
                        ColumnDef::new(Documents::ExtendedFailingTestCount).big_integer(),
                    )
                    .add_column(ColumnDef::new(Documents::FullTestCount).big_integer())
                    .add_column(ColumnDef::new(Documents::FullFailingTestCount).big_integer())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Documents::Table)
                    .drop_column(Documents::BasicTestCount)
                    .drop_column(Documents::BasicFailingTestCount)
                    .drop_column(Documents::ExtendedTestCount)
                    .drop_column(Documents::ExtendedFailingTestCount)
                    .drop_column(Documents::FullTestCount)
                    .drop_column(Documents::FullFailingTestCount)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Documents {
    Table,
    BasicTestCount,
    BasicFailingTestCount,
    ExtendedTestCount,
    ExtendedFailingTestCount,
    FullTestCount,
    FullFailingTestCount,
}

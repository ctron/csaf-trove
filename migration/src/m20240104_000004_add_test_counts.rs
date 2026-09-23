use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for col in [
            ColumnDef::new(Documents::BasicTestCount).big_integer().to_owned(),
            ColumnDef::new(Documents::BasicFailingTestCount).big_integer().to_owned(),
            ColumnDef::new(Documents::ExtendedTestCount).big_integer().to_owned(),
            ColumnDef::new(Documents::ExtendedFailingTestCount).big_integer().to_owned(),
            ColumnDef::new(Documents::FullTestCount).big_integer().to_owned(),
            ColumnDef::new(Documents::FullFailingTestCount).big_integer().to_owned(),
        ] {
            manager
                .alter_table(
                    Table::alter()
                        .table(Documents::Table)
                        .add_column(col)
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for col in [
            Documents::BasicTestCount,
            Documents::BasicFailingTestCount,
            Documents::ExtendedTestCount,
            Documents::ExtendedFailingTestCount,
            Documents::FullTestCount,
            Documents::FullFailingTestCount,
        ] {
            manager
                .alter_table(
                    Table::alter()
                        .table(Documents::Table)
                        .drop_column(col)
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
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

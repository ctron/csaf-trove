//! Persistence for exact distribution membership discovered from remote indexes.

use super::Storage;
use crate::pipeline::membership::DistributionMembership;
use anyhow::Result;
use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait, Value};
use std::collections::BTreeSet;

impl Storage {
    /// Replaces successful indexes atomically, retaining the last known membership on errors or skips.
    pub async fn save_distribution_membership(
        &self,
        domain: &str,
        membership: &DistributionMembership,
    ) -> Result<()> {
        let db = self.db.get(domain).await?;
        let txn = db.begin().await?;
        for (distribution, documents) in membership {
            let previous = txn
                .query_all_raw(Statement::from_sql_and_values(
                    DbBackend::Sqlite,
                    "SELECT document_url FROM distribution_membership WHERE distribution_url = ?",
                    [distribution.clone().into()],
                ))
                .await?
                .into_iter()
                .map(|row| row.try_get("", "document_url"))
                .collect::<Result<BTreeSet<String>, _>>()?;

            // Avoid rewriting large, unchanged indexes; keep batches below SQLite's bind limit.
            let removed: Vec<_> = previous.difference(documents).collect();
            for chunk in removed.chunks(400) {
                let mut values = vec![Value::from(distribution.clone())];
                values.extend(chunk.iter().map(|url| Value::from((*url).clone())));
                txn.execute_raw(Statement::from_sql_and_values(DbBackend::Sqlite,
                    format!("DELETE FROM distribution_membership WHERE distribution_url = ? AND document_url IN ({})",
                        vec!["?"; chunk.len()].join(",")), values,
                )).await?;
            }
            let added: Vec<_> = documents.difference(&previous).collect();
            for chunk in added.chunks(400) {
                let values = chunk.iter().flat_map(|url| {
                    [
                        Value::from(distribution.clone()),
                        Value::from((*url).clone()),
                    ]
                });
                txn.execute_raw(Statement::from_sql_and_values(DbBackend::Sqlite,
                    format!("INSERT INTO distribution_membership (distribution_url, document_url) VALUES {}",
                        vec!["(?, ?)"; chunk.len()].join(",")), values,
                )).await?;
            }
        }
        txn.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;

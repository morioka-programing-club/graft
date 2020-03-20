use std::ops::Deref;
use tokio_postgres::{Error, Row, Client, Statement, types::ToSql};
use async_trait::async_trait;

pub fn is_username(name: &str) -> bool {
	name.starts_with("@")
}

#[async_trait]
pub trait CachedDB : Deref<Target = Client> {
	async fn prepare(&self, query: &str) -> Result<Statement, Error>;

	async fn cquery(&self, query: &str, params: &'_ [&'_ (dyn ToSql + Sync)]) -> Result<Vec<Row>, Error> {
		self.query(&self.prepare(query).await?, params).await
	}

	async fn cexecute(&self, query: &str, params: &'_ [&'_ (dyn ToSql + Sync)]) -> Result<u64, Error> {
		self.execute(&self.prepare(query).await?, params).await
	}
}

#[async_trait]
impl CachedDB for deadpool_postgres::ClientWrapper {
	async fn prepare(&self, query: &str) -> Result<Statement, Error> {
		self.prepare(query).await
	}
}
use std::sync::Arc;
use serde_json::Value;
use std::error::Error;
use tokio_postgres::{NoTls, Row, row::RowIndex};
use tokio_postgres::types::{Type, Kind, IsNull, ToSql, to_sql_checked};
use std::fmt::Display;
use futures::future::{self, TryFutureExt};
use chrono::{DateTime, Utc};
use actix_web::http::StatusCode;
use actix_web::error::{self, Error as ActixError};
use async_recursion::async_recursion;
use ::config::Environment;
use deadpool_postgres::{Pool, Client};

pub mod statements {
	#![allow(non_upper_case_globals)]
	pub const get_inbox: &str = "SELECT * FROM objects WHERE id IN (SELECT message FROM recieved WHERE actor = $1) ORDER BY published;";
	pub const get_outbox: &str = "SELECT * FROM objects WHERE id IN (SELECT message FROM sent WHERE actor = $1) ORDER BY published;";
	pub const get_post: &str = "SELECT * FROM objects WHERE id = $1;";
	pub const get_senders: &str = "SELECT actor FROM sent WHERE message = $1;";
	pub const get_recievers: &str = "SELECT actor FROM recieved WHERE message = $1;";
	pub const get_changelog: &str = "SELECT id FROM activities WHERE object = $1;";
	pub const create_message: &str = "INSERT INTO objects (content, published) VALUES ($1, $2) RETURNING id;";
	pub const version_message: &str = "INSERT INTO messages_changes (version) VALUES ($1) RETURNING id;";
	pub const update_message: &str = "INSERT INTO objects (content, time, changelog) = ($2, $3, $4) WHERE id = $1 RETURNING id;";
	pub const add_message_version: &str = "INSERT INTO messages_changes (id, version) VALUES ($1, $2);";
	pub const add_sender: &str = "INSERT INTO messages_sent (actor, message) VALUES ($2, $1);";
	pub const add_reciever: &str = "INSERT INTO messages_recieved (actor, message) VALUES ($2, $1);";
	pub const create_actor: &str = "INSERT INTO actors (actortype, id) VALUES ($1, $2);";
	pub const delete_actor: &str = "DELETE FROM actors WHERE actortype = $2 AND id = $1;";
}

use crate::activitypub_util::{unwrap_short_vec, MaybeUnwrapped, format_timestamp_rfc3339_seconds_omitted};
use self::statements::*;

lazy_static! {
	pub static ref pool: Pool = ::config::Config::new().merge(Environment::new()).unwrap().clone()
		.try_into::<deadpool_postgres::Config>().unwrap().create_pool(NoTls).unwrap();
}

#[derive(Debug)]
pub enum ActorVariant {
	User,
	Group
}

// Manually expanded https://github.com/sfackler/rust-postgres-derive since it didn't work with tokio-postgres
impl ToSql for ActorVariant {
	fn to_sql(&self, _type: &Type, buf: &mut bytes::BytesMut) -> Result<IsNull, Box<dyn Error + Sync + Send>> {
		let s = match self {
        	ActorVariant::User => "member",
			ActorVariant::Group => "organization"
    	};

    	buf.extend_from_slice(s.as_bytes());
    	Ok(IsNull::No)
	}

    fn accepts(type_: &Type) -> bool {
		if type_.name() != "actors_available" {
            return false;
        }

        match *type_.kind() {
            Kind::Enum(ref variants) => {
                if variants.len() != 2 {
                    return false;
                }

                variants.iter().all(|v| {
                    match &**v {
                        "member" => true,
                        "organization" => true,
                        _ => false
                    }
                })
            }
            _ => false
        }
    }

    to_sql_checked!();
}

pub fn into_value<I>(row: &Row, name: &I, col_type: &Type) -> Value
	where I: RowIndex + Display
{
	macro_rules! from_sql {
		($(($sql_type:ident, $type_to:ty)),*) => {
			match col_type {
				$(&Type::$sql_type => row.get::<&I, Option<$type_to>>(name).map_or(Value::Null, |v| v.into()),)*
				_ => panic!("Specified SQL cell's type is not compatible to JSON")
			}
		}
	}
	if col_type == &Type::TIMESTAMPTZ { format_timestamp_rfc3339_seconds_omitted(row.get::<&I, DateTime<Utc>>(name)).into() }
	else {
		from_sql![
			(CHAR, i8),
			(INT2, i16),
			(INT4, i32),
			(INT8, i64),
			(OID, u32),
			(FLOAT4, f32),
			(FLOAT8, f64),
			(BYTEA, &[u8]),
			(TEXT, &str),
			(BOOL, bool),
			(TEXT_ARRAY, Vec<&str>)
		]
	}
}

pub fn expect_single<T>(err_none: &'static str, err_multi: &'static str) -> Box<dyn Fn(Vec<T>) -> Result<T, ActixError>> {
	Box::new(move |result| {
		match unwrap_short_vec(result) {
			MaybeUnwrapped::Single(value) => Ok(value),
			MaybeUnwrapped::None => Err(error::ErrorNotFound(err_none)),
			MaybeUnwrapped::Multiple(_) => Err(error::ErrorInternalServerError(err_multi))
		}
	})
}

#[async_recursion]
pub async fn process_senders(json: Value, id: i64, db: Arc<Client>) -> Result<(), ActixError> {
	match json {
		Value::String(str) => {
			db.execute(add_sender, &[&id, &str])
				.map_ok(|_| ())
				.map_err(error::ErrorInternalServerError).await
		},
		Value::Object(obj) => {
			db.execute(add_sender, &[&id, &obj["id"].as_str()])
				.map_ok(|_| ())
				.map_err(error::ErrorInternalServerError).await
		},
		Value::Array(arr) => future::try_join_all(arr.to_owned().into_iter().map(move |el| process_senders(el, id, db.clone())))
			.map_ok(|_| ()).map_err(|e| error::InternalError::new(e, StatusCode::INTERNAL_SERVER_ERROR).into()).await,
		_ => Err(error::ErrorBadRequest("Invaild actor"))
	}
}

#[async_recursion]
pub async fn process_recievers(json: Value, id: i64, db: Arc<Client>) -> Result<(), ActixError> {
	match json {
		Value::String(str) => {
			db.execute(add_reciever, &[&id, &str])
				.map_ok(|_| ())
				.map_err(error::ErrorInternalServerError).await
		},
		Value::Object(obj) => {
			db.execute(add_reciever, &[&id, &obj["id"].as_str()])
				.map_ok(|_| ())
				.map_err(error::ErrorInternalServerError).await
		},
		Value::Array(arr) => future::try_join_all(arr.to_owned().into_iter().map(move |el| process_recievers(el, id, db.clone())))
			.map_ok(|_| ()).map_err(|e| error::InternalError::new(e, StatusCode::INTERNAL_SERVER_ERROR).into()).await,
		_ => Err(error::ErrorBadRequest("Invaild actor"))
	}
}

pub async fn get_senders_and_recievers(db: &mut Client, id: i64) -> Result<(Vec<Value>, Vec<Value>), tokio_postgres::Error> {
	future::try_join(
		db.query(get_senders, &[&id])
			.map_ok(|senders| senders.into_iter().map(|sender| into_value(&sender, &0, &Type::TEXT)).collect()),
		db.query(get_recievers, &[&id])
			.map_ok(|recievers| recievers.into_iter().map(|reciever| into_value(&reciever, &0, &Type::TEXT)).collect())
	).await
}

pub async fn get() -> Result<Client, deadpool_postgres::PoolError> {
	pool.get().await
}
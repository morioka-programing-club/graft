use actix_web::{web, error, http::StatusCode, Error as ActixError};
use std::io::{self, ErrorKind};
use futures_locks::Mutex;
use serde_json::Value;
use std::error::Error;
use actix::prelude::*;
use tokio_postgres::{connect, NoTls, Statement, Client, Row, row::RowIndex, types::{Type, Kind, IsNull, ToSql}};
use std::fmt::Display;
use futures::future;
use chrono::{DateTime, Utc};

use crate::activitypub_util::{unwrap_short_vec, MaybeUnwrapped, format_timestamp_rfc3339_seconds_omitted};

#[derive(Debug)]
pub enum ActorVariant {
	User,
	Group
}

// Manually expanded https://github.com/sfackler/rust-postgres-derive since it didn't work with tokio-postgres
impl ToSql for ActorVariant {
	fn to_sql(&self, _type: &Type, buf: &mut Vec<u8>) -> Result<IsNull, Box<Error + Sync + Send>> {
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

    fn to_sql_checked(&self, type_: &Type, buf: &mut Vec<u8>) -> Result<IsNull, Box<Error + Sync + Send>> {
		self.to_sql(type_, buf)
	}
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

pub fn expect_single<T>(err_none: &'static str, err_multi: &'static str) -> Box<Fn(Vec<T>) -> Result<T, ActixError>> {
	Box::new(move |result| {
		match unwrap_short_vec(result) {
			MaybeUnwrapped::Single(value) => Ok(value),
			MaybeUnwrapped::None => Err(error::ErrorNotFound(err_none)),
			MaybeUnwrapped::Multiple(_) => Err(error::ErrorInternalServerError(err_multi))
		}
	})
}

pub struct Db {
	client: Client,
	statements: Statements
}

impl Actor for Db {
    type Context = Context<Self>;
}

impl Db {
	pub fn get(&mut self) -> (&mut Client, &Statements) {
		(&mut self.client, &self.statements)
	}
}

// TODO: statement declarations are repeated 3 times, macro is preferred
pub struct Statements {
	pub get_inbox: Statement,
	pub get_outbox: Statement,
	pub get_message: Statement,
	pub get_senders: Statement,
	pub get_recievers: Statement,
	pub get_changelog: Statement,
	pub create_message: Statement,
	pub version_message: Statement,
	pub update_message: Statement,
	pub add_message_version: Statement,
	pub add_sender: Statement,
	pub add_reciever: Statement,
	pub create_actor: Statement,
	pub delete_actor: Statement
}

pub type DbWrapper = web::Data<Mutex<Db>>;

pub fn process_senders(json: Value, id: i64, db: DbWrapper) -> Box<Future<Item = (), Error = ActixError>> {
	match json {
		Value::String(str) => {
			Box::new(db.lock().from_err().join(future::ok(str)).and_then(move |(mut db_locked, str)| {
				let (client, statements) = db_locked.get();
				client.execute(&statements.add_sender, &[&id, &str])
					.map(|_| ()).map_err(error::ErrorInternalServerError)
			}))
		},
		Value::Object(obj) => {
			Box::new(db.lock().from_err().join(future::ok(obj)).and_then(move |(mut db_locked, obj)| {
				let (client, statements) = db_locked.get();
				client.execute(&statements.add_sender, &[&id, &obj["id"].as_str()])
					.map(|_| ()).map_err(error::ErrorInternalServerError)
			}))
		},
		Value::Array(arr) => Box::new(
			future::join_all(arr.to_owned().into_iter().map(move |el| process_senders(el, id, db.clone()))).map(|_| ())
				.map_err(|e| error::InternalError::new(e, StatusCode::INTERNAL_SERVER_ERROR).into())
		),
		_ => Box::new(future::err(error::ErrorBadRequest("Invaild actor")))
	}
}

pub fn process_recievers(json: Value, id: i64, db: DbWrapper) -> Box<Future<Item = (), Error = ActixError>> {
	match json {
		Value::String(str) => {
			Box::new(db.lock().from_err().join(future::ok(str)).and_then(move |(mut db_locked, str)| {
				let (client, statements) = db_locked.get();
				client.execute(&statements.add_reciever, &[&id, &str])
					.map(|_| ()).map_err(error::ErrorInternalServerError)
			}))
		},
		Value::Object(obj) => {
			Box::new(db.lock().from_err().join(future::ok(obj)).and_then(move |(mut db_locked, obj)| {
				let (client, statements) = db_locked.get();
				client.execute(&statements.add_reciever, &[&id, &obj["id"].as_str()])
					.map(|_| ()).map_err(error::ErrorInternalServerError)
			}))
		},
		Value::Array(arr) => Box::new(
			future::join_all(arr.to_owned().into_iter().map(move |el| process_recievers(el, id, db.clone()))).map(|_| ())
				.map_err(|e| error::InternalError::new(e, StatusCode::INTERNAL_SERVER_ERROR).into())
		),
		_ => Box::new(future::err(error::ErrorBadRequest("Invaild actor")))
	}
}

pub fn get_senders_and_recievers(client: &mut Client, statements: &Statements, id: i64) -> impl Future<Item = (Vec<Value>, Vec<Value>), Error = tokio_postgres::Error> {
	client.query(&statements.get_senders, &[&id])
		.map(|sender| into_value(&sender, &0, &Type::TEXT))
		.collect()
		.join(
			client.query(&statements.get_recievers, &[&id])
				.map(|reciever| into_value(&reciever, &0, &Type::TEXT))
				.collect()
		)
}

pub fn init(user_name: &str) -> Box<Future<Item = DbWrapper, Error = io::Error>> {
	Box::new(
		connect(&(String::from("postgres://") + user_name + "@localhost/graft"), NoTls)
			.map_err(|e| io::Error::new(ErrorKind::Other, e))
			.and_then(move |(mut cl, conn)| {
				Arbiter::spawn(conn.map_err(|e| panic!("{}", e)));
				future::join_all(vec![
					// Insert SQL statements here
					cl.prepare("SELECT * FROM messages WHERE id IN (SELECT message FROM messages_recieved WHERE actor = $1) ORDER BY time;"), // get inbox
					cl.prepare("SELECT * FROM messages WHERE id IN (SELECT message FROM messages_sent WHERE actor = $1) ORDER BY time;"), // get outbox
					cl.prepare("SELECT * FROM messages WHERE id = $1;"), // get message
					cl.prepare("SELECT actor FROM messages_sent WHERE message = $1;"), // get senders
					cl.prepare("SELECT actor FROM messages_recieved WHERE message = $1;"), // get recievers
					cl.prepare("SELECT version FROM messages_changes WHERE id = $1;"), // get changelog
					cl.prepare("INSERT INTO messages (content, time, changelog) VALUES ($1, $2, $3) RETURNING id;"), // create message
					cl.prepare("INSERT INTO messages_changes (version) VALUES ($1) RETURNING id;"), // version message
					cl.prepare("UPDATE messages SET (content, time, changelog) = ($2, $3, $4) WHERE id = $1 RETURNING id;"), // update message
					cl.prepare("INSERT INTO messages_changes (id, version) VALUES ($1, $2);"), // add message version
					cl.prepare("INSERT INTO messages_sent (actor, message) VALUES ($2, $1);"), // add sender
					cl.prepare("INSERT INTO messages_recieved (actor, message) VALUES ($2, $1);"), // add reciever
					cl.prepare("INSERT INTO actors (actortype, id) VALUES ($1, $2);"), // create actor
					cl.prepare("DELETE FROM actors WHERE actortype = $2 AND id = $1;") // delete actor
				]).and_then(move |statements| {
					println!("SQL Statements prepared successfully");
					let mut iter = statements.into_iter();
					Ok(web::Data::new(Mutex::new(Db {
						client: cl,
						statements: Statements {
							get_inbox: iter.next().unwrap(),
							get_outbox: iter.next().unwrap(),
							get_message: iter.next().unwrap(),
							get_senders: iter.next().unwrap(),
							get_recievers: iter.next().unwrap(),
							get_changelog: iter.next().unwrap(),
							create_message: iter.next().unwrap(),
							version_message: iter.next().unwrap(),
							update_message: iter.next().unwrap(),
							add_message_version: iter.next().unwrap(),
							add_sender: iter.next().unwrap(),
							add_reciever: iter.next().unwrap(),
							create_actor: iter.next().unwrap(),
							delete_actor: iter.next().unwrap()
						}
					})))
				}).map_err(|e| io::Error::new(ErrorKind::Other, e))
			})
	)
}
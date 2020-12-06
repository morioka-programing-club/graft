use mongodb::Database;
use mongodb::options::FindOneOptions;
use serde_json::Value;
use futures::future::{self, TryFutureExt};
use actix_web::http::{StatusCode, uri::Uri};
use actix_web::error::{self, Error as ActixError};
use async_recursion::async_recursion;
use std::any::type_name;
use serde::Deserialize;
use bson::{from_bson, doc, Bson, Document};

use crate::util::rename_property;

#[derive(Debug)]
pub enum ActorVariant {
	User,
	Group
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
pub async fn process_senders(json: Value, id: i64, db: Database) -> Result<(), ActixError> {
	match json {
		Value::String(str) => {
			db.cexecute(add_sender, &[&id, &str])
				.map_ok(|_| ())
				.map_err(error::ErrorInternalServerError).await
		},
		Value::Object(obj) => {
			db.cexecute(add_sender, &[&id, &obj["id"].as_str()])
				.map_ok(|_| ())
				.map_err(error::ErrorInternalServerError).await
		},
		Value::Array(arr) => future::try_join_all(arr.to_owned().into_iter().map(move |el| process_senders(el, id, db.clone())))
			.map_ok(|_| ()).map_err(|e| error::InternalError::new(e, StatusCode::INTERNAL_SERVER_ERROR).into()).await,
		_ => Err(error::ErrorBadRequest("Invaild actor"))
	}
}

#[async_recursion]
pub async fn process_recievers(json: Value, id: i64, db: Database) -> Result<(), ActixError> {
	match json {
		Value::String(str) => {
			db.cexecute(add_reciever, &[&id, &str])
				.map_ok(|_| ())
				.map_err(error::ErrorInternalServerError).await
		},
		Value::Object(obj) => {
			db.cexecute(add_reciever, &[&id, &obj["id"].as_str()])
				.map_ok(|_| ())
				.map_err(error::ErrorInternalServerError).await
		},
		Value::Array(arr) => future::try_join_all(arr.to_owned().into_iter().map(move |el| process_recievers(el, id, db.clone())))
			.map_ok(|_| ()).map_err(|e| error::InternalError::new(e, StatusCode::INTERNAL_SERVER_ERROR).into()).await,
		_ => Err(error::ErrorBadRequest("Invaild actor"))
	}
}

pub async fn get_object<'de, T: Deserialize<'de>>(uri: &str, db: Database) -> Result<T, ActixError>
{
	let mut doc = db.collection("objects").find_one(doc!{"_id": uri}, FindOneOptions::default()).await
		.map_err(error::ErrorInternalServerError)?
		.ok_or(error::ErrorNotFound("Not Found"))?;
	rename_property(&mut doc, "_id", "id".to_string())?;
	from_bson(Bson::Document(doc)).map_err(
		|_| error::ErrorNotFound(String::from("The object found could not be converted into ") + type_name::<T>())
	)
}
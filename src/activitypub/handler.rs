use std::iter::once;
use std::borrow::Cow;

use mongodb::Client;
use actix_web::{web::{Data, Json, Path}, HttpRequest, Responder, http::StatusCode};
use actix_web::error::{Error as ActixError, ErrorBadRequest};
use chrono::Utc;
use serde_json::{Map, Value, json};
use serde::de::{Deserialize, IntoDeserializer, value::StrDeserializer};

use crate::db::{get, get_record, insert};
use crate::util::*;
use crate::error::internal_error;
use super::*;
use super::jsonld::*;
use super::strip::*;

pub async fn account(req: HttpRequest, id: ObjectId, db: Data<Client>) -> Result<String, ActixError> {
	let mut account = get(&id, &db).await?;
	let options = json_ld_options(&req)?;
	let context = context(&account, req.head())?;
	account = unstrip_actor(account, &options).await.map_err(internal_error)?;
	serde_json::to_string(&compact_object(account, context, &options).await.map_err(internal_error)?).map_err(internal_error)
}

pub async fn inbox(req: HttpRequest, id: ObjectId, db: Data<Client>) -> Result<String, ActixError> {
	let mut inbox = Map::new();
	let options = json_ld_options(&req)?;
	let context = context(&inbox, req.head())?;
	inbox = unstrip_object(inbox, &options).await.map_err(internal_error)?;

	//inbox["items"] = Value::Array(get(&id, &db.collection("activities")).await?);

	Ok(serde_json::to_string(&compact_object(inbox, context, &options).await.map_err(internal_error)?).map_err(internal_error)?)
}

pub async fn outbox(req: HttpRequest, id: ObjectId, db: Data<Client>) -> Result<String, ActixError> {
	let mut outbox = Map::new();
	let options = json_ld_options(&req)?;
	let context = context(&outbox, req.head())?;
	outbox = unstrip_object(outbox, &options).await.map_err(internal_error)?;

	//outbox["items"] = Value::Array(get(&id, &db.collection("activities")).await?);

	Ok(serde_json::to_string(&compact_object(outbox, context, &options).await.map_err(internal_error)?).map_err(internal_error)?)
}

pub async fn post(req: HttpRequest, id: ObjectId, db: Data<Client>) -> Result<String, ActixError> {
	let mut post = get(&id, &db).await?;
	let options = json_ld_options(&req)?;
	let context = context(&post, req.head())?;
	post = unstrip_object(post, &options).await.map_err(internal_error)?;
	Ok(serde_json::to_string(&compact_object(post, context, &options).await.map_err(internal_error)?).map_err(internal_error)?)
}

pub async fn activity(req: HttpRequest, id: ObjectId, db: Data<Client>) -> Result<String, ActixError> {
	let mut post = get(&id, &db).await?;
	let options = json_ld_options(&req)?;
	let context = context(&post, req.head())?;
	post = unstrip_object(post, &options).await.map_err(internal_error)?;
	Ok(serde_json::to_string(&compact_object(post, context, &options).await.map_err(internal_error)?).map_err(internal_error)?)
}

pub async fn record(req: HttpRequest, path: Path<((), String)>, id: ObjectId, db: Data<Client>) -> Result<String, ActixError> {
	// Omitting second is not supported by chrono. This workaround relies on `parse_rfc3339` parsing the field in order.
	let mut time = chrono::format::Parsed::new();
	let _ = chrono::format::parse(&mut time, &path.1, [chrono::format::Item::Fixed(chrono::format::Fixed::RFC3339)].iter());

	// Seconds and milis default to max to make sure object in that exact time will match
	time.second = time.second.or(Some(59));
	time.nanosecond = time.nanosecond.or(Some(999_000_000));
	let time = time.to_datetime_with_timezone(&Utc).map_err(ErrorBadRequest)?;

	let mut post = get_record(&id, &time, &db).await?;
	let options = json_ld_options(&req)?;
	let context = context(&post, req.head())?;
	post = unstrip_object(post, &options).await.map_err(internal_error)?;
	Ok(serde_json::to_string(&compact_object(post, context, &options).await.map_err(internal_error)?).map_err(internal_error)?)
}

pub async fn get_changelog(req: HttpRequest, id: ObjectId, db: Data<Client>) -> Result<String, ActixError> {
	let mut history = get(&id, &db).await?;
	let options = json_ld_options(&req)?;
	let context = context(&history, req.head())?;
	history = unstrip_object(history, &options).await.map_err(internal_error)?;
	Ok(serde_json::to_string(&compact_object(history, context, &options).await.map_err(internal_error)?).map_err(internal_error)?)
}

pub async fn create_account(req: HttpRequest, mut account: Json<Map<String, Value>>, db: Data<Client>) ->
	Result<impl Responder, ActixError>
{
	let id = generate_id().to_string();
	let url = req.url_for("account", [
		&*account.get("name").map_or(Cow::Borrowed(""), |name| Cow::Owned(name.to_string() + "-")),
		&id
	]).map_err(internal_error)?.to_string();
	let options = json_ld_options(&req)?;
	let context = context(&account, req.head())?;
	account.insert("id".to_string(), id.into());
	let timestamp = datetime(Utc::now());
	account.insert(ns!(as:published).to_string(), timestamp.clone());
	account.insert(ns!(as:updated).to_string(), timestamp);
	insert(&strip_object(account.into_inner(), context, &options).await.map_err(ErrorBadRequest)?, &db).await?;
	Ok("".to_string().with_status(StatusCode::CREATED).with_header(("Location", url)))
}

pub async fn submit(req: HttpRequest, ref actor: ObjectId, json: Json<Map<String, Value>>, db: Data<Client>) -> impl Responder {
	let options = json_ld_options(&req)?;
	let context = context(&json, req.head())?;
	let mut json = expand_object(json.into_inner(), &options).await.map_err(ErrorBadRequest)?;
	let timestamp = datetime(Utc::now());

	let ty = json.get("@type").ok_or(ErrorBadRequest("missing `type`"))?.as_array().expect("expanded object")
		.iter().map(|ty| ty.as_str()).find_map(|ty|
	{
		if let Some(ty) = ty {
			let deserializer: StrDeserializer<serde::de::value::Error> = ty.into_deserializer();
			SupportedActivity::deserialize(deserializer).ok().map(Ok)
		} else {
			Some(Err(ErrorBadRequest("value(s) of `type` must be string")))
		}
	}).transpose()?;
	if let Some(ty) = ty { // Don't consider compound types for now
		let object = json.remove(ns!(as:object))
			.and_then(|mut object| object.as_array_mut().map(|object| object.remove(0)))
			.ok_or(ErrorBadRequest("invalid `object`"))?;
		if let Value::Object(mut object) = object {
			use SupportedActivity::*;
			match ty {
				Create => {
					copy_recipients(&json, &mut object);
					copy_recipients(&object, &mut json);
					object.insert(ns!(as:attributedTo).to_string(), json!({"@id": actor.to_string()}));
					object.insert("@id".to_string(), generate_id().to_string().into()); // Overwrite any existing id

					object.insert(ns!(as:published).to_string(), timestamp.clone());
					object.insert(ns!(as:updated).to_string(), timestamp.clone());
					insert(&strip_object(object.clone(), context.clone(), &options).await.map_err(internal_error)?, &db).await?;
					json.insert(ns!(as:object).to_string(), object.into());
				},
				Update => {
					let mut old = get(&get_id(&object)?, &db).await?;
					/*if old.get_str("attributedTo") != Ok(json["actor"].as_str().expect()) {
						return Err(error::ErrorBadRequest("Unauthorized edit"));
					}*/
					object.insert(ns!(as:updated).to_string(), timestamp.clone());
					let object = strip_object(object, context.clone(), &options).await.map_err(internal_error)?;
					for (key, value) in object {
						if key == "id" { continue; }
						if value.is_null() {
							old.remove(&key);
						} else {
							old.insert(key, value);
						}
					}
					insert(&old, &db).await?;
				},
				Delete => {
					let mut tombstone = Map::new();
					tombstone.insert("id".to_string(), (&get_id(&object)?).into());
					tombstone.insert("type".to_string(), "Tombstone".into());
					tombstone.insert("published".to_string(), timestamp.clone());
					tombstone.insert("updated".to_string(), timestamp.clone());
					tombstone.insert("deleted".to_string(), timestamp.clone());
					insert(&strip_object(tombstone, context.clone(), &options).await.map_err(internal_error)?, &db).await?;
				},
			}
		} else {
			return Err(ErrorBadRequest("invalid `object`"));
		}
		json.insert(ns!(as:published).to_string(), timestamp.clone());
		json.insert(ns!(as:updated).to_string(), timestamp);
		let id = generate_id().to_string();
		let url = req.url_for("activity", once(&id)).map_err(internal_error)?.to_string();
		json.insert("@id".to_string(), id.into());
		json.insert(ns!(as:actor).to_string(), json!({ "@id": actor.to_string() }));
		insert(&strip_object(json, context, &options).await.map_err(internal_error)?, &db).await?;
		Ok("".to_string().with_status(StatusCode::CREATED).with_header(("Location", url)))
	} else {
		//let create_activity = Create::new(, json);
		todo!()
	}
}

pub async fn delivery() -> Result<String, ActixError> {
	todo!()
}
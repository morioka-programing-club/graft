use std::borrow::Cow;
use std::iter::once;

use actix_web::error::{Error as ActixError, ErrorBadRequest, ErrorNotFound};
use actix_web::http::StatusCode;
use actix_web::web::{Data, Json, Path};
use actix_web::{HttpRequest, Responder};
use chrono::Utc;
use json_trait::{json, BuildableJson};
use mongodb::Client;
use serde::de::value::StrDeserializer;
use serde::de::{Deserialize, IntoDeserializer};
use serde_json::{Map, Value};

use super::jsonld::*;
use super::strip::*;
use super::*;
use crate::db::{get, get_record, insert};
use crate::error::internal_error;
use crate::util::*;

pub async fn account(req: HttpRequest, id: ObjectId, db: Data<Client>) -> Result<Json<Map<String, Value>>, ActixError> {
	let mut account = get(&id, &db).await?.ok_or(ErrorNotFound(""))?;
	let options = json_ld_options(&req)?;
	let context = context(&account, req.head())?;
	account = unstrip_actor(&account, &options).await.map_err(internal_error)?;
	Ok(Json(compact_object(&account, context, &options).await.map_err(internal_error)?))
}

pub async fn inbox(req: HttpRequest, id: ObjectId, db: Data<Client>) -> Result<Json<Map<String, Value>>, ActixError> {
	let mut inbox = Map::new();
	let options = json_ld_options(&req)?;
	let context = context(&inbox, req.head())?;
	inbox = unstrip_object(&inbox, &options).await.map_err(internal_error)?;

	// inbox["items"] = Value::Array(get(&id, &db.collection("activities")).await?);

	Ok(Json(compact_object(&inbox, context, &options).await.map_err(internal_error)?))
}

pub async fn outbox(req: HttpRequest, id: ObjectId, db: Data<Client>) -> Result<Json<Map<String, Value>>, ActixError> {
	let mut outbox = Map::new();
	let options = json_ld_options(&req)?;
	let context = context(&outbox, req.head())?;
	outbox = unstrip_object(&outbox, &options).await.map_err(internal_error)?;

	// outbox["items"] = Value::Array(get(&id, &db.collection("activities")).await?);

	Ok(Json(compact_object(&outbox, context, &options).await.map_err(internal_error)?))
}

pub async fn post(req: HttpRequest, id: ObjectId, db: Data<Client>) -> Result<Json<Map<String, Value>>, ActixError> {
	let mut post = get(&id, &db).await?.ok_or(ErrorNotFound(""))?;
	let options = json_ld_options(&req)?;
	let context = context(&post, req.head())?;
	post = unstrip_object(&post, &options).await.map_err(internal_error)?;
	Ok(Json(compact_object(&post, context, &options).await.map_err(internal_error)?))
}

pub async fn activity(req: HttpRequest, id: ObjectId, db: Data<Client>) -> Result<Json<Map<String, Value>>, ActixError> {
	let mut post = get(&id, &db).await?.ok_or(ErrorNotFound(""))?;
	let options = json_ld_options(&req)?;
	let context = context(&post, req.head())?;
	post = unstrip_object(&post, &options).await.map_err(internal_error)?;
	Ok(Json(compact_object(&post, context, &options).await.map_err(internal_error)?))
}

pub async fn record(req: HttpRequest, path: Path<((), String)>, id: ObjectId, db: Data<Client>) -> Result<Json<Map<String, Value>>, ActixError> {
	// Omitting second is not supported by chrono. This workaround relies on `parse_rfc3339` parsing the field in order.
	let mut time = chrono::format::Parsed::new();
	let _ = chrono::format::parse(&mut time, &path.1, [chrono::format::Item::Fixed(chrono::format::Fixed::RFC3339)].iter());

	// Seconds and milis default to max to make sure object in that exact time will match
	time.second = time.second.or(Some(59));
	time.nanosecond = time.nanosecond.or(Some(999_000_000));
	let time = time.to_datetime_with_timezone(&Utc).map_err(ErrorBadRequest)?;

	let mut post = get_record(&id, &time, &db).await?.ok_or(ErrorNotFound(""))?;
	let options = json_ld_options(&req)?;
	let context = context(&post, req.head())?;
	post = unstrip_object(&post, &options).await.map_err(internal_error)?;
	Ok(Json(compact_object(&post, context, &options).await.map_err(internal_error)?))
}

pub async fn get_changelog(req: HttpRequest, id: ObjectId, db: Data<Client>) -> Result<Json<Map<String, Value>>, ActixError> {
	let mut history = get(&id, &db).await?.ok_or(ErrorNotFound(""))?;
	let options = json_ld_options(&req)?;
	let context = context(&history, req.head())?;
	history = unstrip_object(&history, &options).await.map_err(internal_error)?;
	Ok(Json(compact_object(&history, context, &options).await.map_err(internal_error)?))
}

pub async fn create_account(req: HttpRequest, mut account: Json<Map<String, Value>>, db: Data<Client>) -> Result<impl Responder, ActixError> {
	let id = generate_id().to_string();
	let url = req
		.url_for("account", [
			&*if let Some(name) = account.get("name") {
				Cow::Owned(name.as_str().ok_or(ErrorBadRequest("`name` must be string"))?.to_string() + "-")
			} else {
				Cow::Borrowed("")
			},
			&id
		])
		.map_err(internal_error)?
		.to_string();
	let options = json_ld_options(&req)?;
	let context = context(&account, req.head())?;
	account.insert("id".to_string(), id.into());
	let timestamp = datetime(Utc::now());
	account.insert(ns!(as:published).to_string(), timestamp.clone());
	account.insert(ns!(as:updated).to_string(), timestamp);
	insert(&strip_object(&account, context, &options).await.map_err(ErrorBadRequest)?, &db).await?;
	Ok("".to_string().customize().with_status(StatusCode::CREATED).insert_header(("Location", url)))
}

pub async fn submit(req: HttpRequest, ref actor: ObjectId, json: Json<Map<String, Value>>, db: Data<Client>) -> impl Responder {
	let options = json_ld_options(&req)?;
	let context = context(&json, req.head())?;
	let ref mut json = expand_object(&json, &options).await.map_err(ErrorBadRequest)?;
	let timestamp = datetime(Utc::now());

	let ty = json
		.get("@type")
		.ok_or(ErrorBadRequest("missing `type`"))?
		.as_array()
		.expect("expanded object")
		.iter()
		.map(|ty| ty.as_str().expect("expanded object"))
		.find_map(|ty| {
			let deserializer: StrDeserializer<serde::de::value::Error> = ty.into_deserializer();
			SupportedActivity::deserialize(deserializer).ok()
		});
	use SupportedActivity::*;
	if let Some(ty) = ty {
		match ty {
			Create => {
				let mut new_object = Vec::new();
				for mut object in take_objects(json, ns!(as:object)).ok_or(ErrorBadRequest("invalid `object`"))? {
					copy_recipients(json, &mut object);
					copy_recipients(&object, json);
					object.insert(ns!(as:attributedTo).to_string(), json!(Value, {"@id": actor.to_string()}));
					object.insert("@id".to_string(), generate_id().to_string().into()); // Overwrite any existing id

					object.insert(ns!(as:published).to_string(), timestamp.clone());
					object.insert(ns!(as:updated).to_string(), timestamp.clone());
					insert(&strip_object(&object, context.clone(), &options).await.map_err(internal_error)?, &db).await?;
					new_object.push(Value::Object(object));
				}
				json.insert(ns!(as:object).to_string(), Value::Array(new_object));
			}
			Update => {
				for object in get_objects_mut(json, ns!(as:object)).ok_or(ErrorBadRequest("invalid `object`"))? {
					let mut old = get(&get_id(object)?, &db).await?.ok_or(ErrorBadRequest("`object` not found"))?;
					// if old.get_str("attributedTo") != Ok(json["actor"].as_str().expect()) {
					// return Err(error::ErrorBadRequest("Unauthorized edit"));
					// }
					object.insert(ns!(as:updated).to_string(), timestamp.clone());
					for (key, value) in strip_object(object, context.clone(), &options).await.map_err(internal_error)? {
						if key == "id" {
							continue;
						}
						if value.is_null() {
							old.remove(&key);
						} else {
							old.insert(key, value);
						}
					}
					insert(&old.clone(), &db).await?;
					*object = old;
				}
			}
			Delete => {
				for object in get_objects(json, ns!(as:object)).ok_or(ErrorBadRequest("invalid `object`"))? {
					let tombstone = json!(Value, {
						"id": get_id(&object)?.to_string(),
						"type": "Tombstone",
						"published": timestamp.clone(),
						"updated": timestamp.clone(),
						"deleted": timestamp.clone()
					});
					insert(&strip_object(&tombstone, context.clone(), &options).await.map_err(internal_error)?, &db).await?;
				}
			}
			Follow => (), // Follow is processed upon acceptance
			Add => {
				for target in get_objects(json, ns!(as:target)).ok_or(ErrorBadRequest("invalid `target`"))? {
					let mut target = get(&get_id(target)?, &db).await?.ok_or(ErrorBadRequest("`target` not found"))?;
					if !is_collection(&target)? {
						return Err(ErrorBadRequest("`target` must be a collection"));
					}
					if let Some(items) = target.get_mut(ns!(as:items)).map(|items| items.as_array_mut().unwrap()) {
						for object in get_objects(json, ns!(as:object)).ok_or(ErrorBadRequest("invalid `object`"))? {
							items.push(Value::Object(object.clone()));
						}
					} else {
						todo!() // Paging
					}
					target.insert(ns!(as:updated).to_string(), timestamp.clone());
					insert(&strip_object(&target, context.clone(), &options).await.map_err(internal_error)?, &db).await?;
				}
			}
			Remove => {
				let origins = get_objects(json, ns!(as:origin))
					.or_else(|| get_objects(json, ns!(as:target)))
					.ok_or(ErrorBadRequest("invalid `origin`"))?;
				for origin in origins {
					let mut origin = get(&get_id(origin)?, &db).await?.ok_or(ErrorBadRequest("`origin` not found"))?;
					if !is_collection(&origin)? {
						return Err(ErrorBadRequest("`origin` must be a collection"));
					}
					if let Some(items) = origin.remove(ns!(as:items)).map(|items| items.into_array().unwrap()) {
						let mut objects = get_objects(json, ns!(as:object)).ok_or(ErrorBadRequest("invalid `object`"))?.collect::<Vec<_>>();
						origin.insert(
							ns!(as:items).to_string(),
							items
								.into_iter()
								.filter(|item| {
									for (i, object) in objects.iter().enumerate() {
										if item.as_object().expect("expanded object") == *object {
											objects.remove(i);
											return false;
										}
									}
									true
								})
								.collect::<Value>()
						);
					} else {
						todo!() // Paging
					}
				}
			}
			Like => {
				todo!()
			}
			Block => {
				todo!()
			}
			Undo => {
				todo!()
			}
		}
		json.insert(ns!(as:published).to_string(), timestamp.clone());
		json.insert(ns!(as:updated).to_string(), timestamp);
		let id = generate_id().to_string();
		let url = req.url_for("activity", once(&id)).map_err(internal_error)?.to_string();
		json.insert("@id".to_string(), id.into());
		json.insert(ns!(as:actor).to_string(), json!(Value, { "@id": actor.to_string() }));
		insert(&strip_object(json, context, &options).await.map_err(internal_error)?, &db).await?;
		Ok("".to_string().customize().with_status(StatusCode::CREATED).insert_header(("Location", url)))
	} else {
		// let create_activity = Create::new(, json);
		todo!()
	}
}

pub async fn delivery() -> Result<String, ActixError> {
	todo!()
}

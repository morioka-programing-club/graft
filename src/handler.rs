use actix_web::{web, HttpRequest, Responder, error::{self, Error as ActixError}};
use actix_web::http::uri::{Uri, Parts, PathAndQuery};
use activitypub::{actor, collection};
use chrono::{Utc, DateTime};
use futures::future::TryFutureExt;
use serde_json::{Map, Value};
use std::sync::Arc;

use crate::db::{self, process_senders, process_recievers, expect_single};
use crate::activitypub_util::{unwrap_short_vec, message_to_json};
use crate::util::is_username;
use crate::db::statements::*;

const PROTOCOL_HOST: &str = "https://localhost:8088";
const CONTEXT: &str = "http://www.w3.org/ns/activitystreams";

lazy_static! {
	static ref ID: String = "id".to_string();
}

pub async fn group() -> Result<String, ActixError> {
	// Maybe a web interface
	todo!()
}

pub async fn group_json(req: HttpRequest) -> Result<String, ActixError> {
	let uri = req.uri();
	let uri_str = &uri.to_string();
	let mut uri_parts = Parts::from(uri.to_owned());
	let mut actor = actor::Group::default();
	uri_parts.path_and_query = Some(PathAndQuery::from_maybe_shared(
		String::from("/to/") + &req.match_info().query("actorname")
	).unwrap());

	actor.ap_actor_props.inbox = Uri::from_parts(uri_parts).unwrap().to_string().into();
	actor.ap_actor_props.outbox = (uri_str.clone() + "/all").into();
	actor.object_props.id = Some(Value::from(uri_str.to_owned()));
	actor.object_props.context = Some(Value::from(CONTEXT));
    serde_json::to_string(&actor).map_err(error::ErrorInternalServerError)
}

pub async fn inbox(req: HttpRequest) -> Result<String, ActixError> {
	let uri = req.uri().to_string();
	let mut inbox = collection::OrderedCollection::default();
	inbox.object_props.id = Some(Value::from(uri.to_owned()));
	inbox.object_props.context = Some(Value::from(CONTEXT));

	let mut db = db::get().map_err(error::ErrorInternalServerError).await?;
	let mut items: Vec<Map<String, Value>> = db.query(get_inbox, &[
		&(PROTOCOL_HOST.to_owned() + "/of/" + req.match_info().query("actorname"))
	]).map_ok(|rows| rows.into_iter().map(message_to_json).collect())
		.map_err(error::ErrorInternalServerError).await?;
	for item in &mut items {
		let id = item.get_mut(&*ID).unwrap().as_i64().unwrap();
		let (senders, recievers) = db::get_senders_and_recievers(&mut db, id)
			.map_err(error::ErrorInternalServerError).await?;
		item.insert("actor".to_string(), unwrap_short_vec(senders).into());
		item.insert("to".to_string(), unwrap_short_vec(recievers).into());
	}
	inbox.collection_props.items = items.into();
	Ok(serde_json::to_string(&inbox).map_err(|_| panic!("JSON serialization error"))?)
}

pub async fn outbox(req: HttpRequest) -> Result<String, ActixError> {
	let uri = req.uri().to_string();
	let mut outbox = collection::OrderedCollection::default();
	outbox.object_props.id = Some(Value::from(uri.to_owned()));
	outbox.object_props.context = Some(Value::from(CONTEXT));

	let mut db = db::get().map_err(error::ErrorInternalServerError).await?;
	let mut items: Vec<Map<String, Value>> = db.query(get_outbox, &[
		&(PROTOCOL_HOST.to_owned() + "/of/" + req.match_info().query("actorname"))
	]).map_ok(|rows| rows.into_iter().map(message_to_json).collect())
		.map_err(error::ErrorInternalServerError).await?;
	for item in &mut items {
		let id = item.get_mut(&*ID).unwrap().as_i64().unwrap();
		let (senders, recievers) = db::get_senders_and_recievers(&mut db, id)
			.map_err(error::ErrorInternalServerError).await?;
		item.insert("actor".to_string(), unwrap_short_vec(senders).into());
		item.insert("to".to_string(), unwrap_short_vec(recievers).into());
	}
	outbox.collection_props.items = items.into();
	Ok(serde_json::to_string(&outbox).map_err(|_| panic!("JSON serialization error"))?)
}

pub async fn view_post(req: HttpRequest) -> Result<String, ActixError> {
	let id = match i64::from_str_radix(req.match_info().query("id"), 10) {
		Ok(id) => id,
		Err(e) => return Err(error::ErrorBadRequest(e))
	};

	let mut db = db::get().map_err(error::ErrorInternalServerError).await?;
	let mut items = expect_single("No post found for the ID requested", "Internal error")(
		db.query(get_post, &[&id])
		.map_ok(|rows| rows.into_iter().map(message_to_json).collect())
		.map_err(error::ErrorInternalServerError).await?)?;
	let (senders, recievers) = db::get_senders_and_recievers(&mut db, id)
		.map_err(error::ErrorInternalServerError).await?;
	items.insert("actor".to_string(), unwrap_short_vec(senders).into());
	items.insert("to".to_string(), unwrap_short_vec(recievers).into());
	Ok(serde_json::to_string(&items).map_err(|_| panic!("JSON serialization error"))?)
}

pub async fn changelog(req: HttpRequest) -> Result<String, ActixError> {
	let id = match i64::from_str_radix(req.match_info().query("id"), 10) {
		Ok(id) => id,
		Err(e) => return Err(error::ErrorBadRequest(e))
	};

	let db = db::get().map_err(error::ErrorInternalServerError).await?;
	let history = db.query(get_changelog, &[&id])
		.map_ok(|rows| rows.into_iter().map(|row| row.get::<usize, i64>(0).into()).collect())
		.map_err(error::ErrorInternalServerError).await?;
	Ok(serde_json::to_string(&Value::Array(history)).map_err(|_| panic!("JSON serialization error"))?)
}

pub async fn create(req: HttpRequest) -> Result<String, ActixError> {
	let mut name = req.match_info().query("actorname").to_owned();
	let isuser = is_username(&name);
	if isuser {name.remove(0);}

	let db = db::get().map_err(error::ErrorInternalServerError).await?;
	db.execute(create_actor, &[
		&if isuser {db::ActorVariant::User} else {db::ActorVariant::Group},
		&req.uri().to_string()
	]).map_err(error::ErrorInternalServerError).await?;
	Ok(if isuser {"User"} else {"Group"}.to_owned() + " succesfully created")
}

pub async fn submit(mut json: web::Json<Value>) -> Result<String, ActixError> {
	if !json["type"].is_string() {
		return Err(error::ErrorBadRequest("Non-activitystream object recieved"))
	}

	let db = db::get().map_err(error::ErrorInternalServerError).await?;
	match json["type"].as_str().unwrap() {
		"Create" => {
			let id = db.query(create_message, &[
				&json["object"]["content"].as_str(), &Utc::now(), &None::<i64>
			]).map_err(error::ErrorInternalServerError).await?[0].get(0);
			let db = Arc::new(db);
			process_senders(json["actor"].take(), id, db.clone()).map_err(error::ErrorInternalServerError).await?;
			process_recievers(json["to"].take(), id, db).map_err(error::ErrorInternalServerError).await?;
			Ok("".to_string())
		},
		"Update" => {
			let message = expect_single("No post found for the ID requested", "Internal error")(
				db.query(get_post, &[&json["object"]["id"].as_i64()])
					.map_err(error::ErrorInternalServerError).await?)?;
			let id = message.get::<&str, i64>("id");
			let logid_original = message.get::<&str, Option<i64>>("changelog");
			let logid = if let None = logid_original {
				expect_single("Internal error", "Internal error")(
					db.query(version_message, &[&id]).map_err(error::ErrorInternalServerError).await?
				)?.get::<usize, i64>(0)
			} else {
				logid_original.unwrap()
			};
			let old = expect_single("Internal error", "Internal error")(
				db.query(create_message, &[
					&message.get::<&str, &str>("content"), &message.get::<&str, DateTime<Utc>>("time"), &logid
				]).map_err(error::ErrorInternalServerError).await?)?;

			db.query(add_message_version, &[&logid, &old.get::<&str, i64>("id")])
				.map_err(error::ErrorInternalServerError).await?;
			db.query(update_message, &[&id, &json["object"]["content"].as_str(), &Utc::now(), &logid])
				.map_err(error::ErrorInternalServerError).await?;
			Ok("".to_string())
		},
		_ => Err(error::ErrorBadRequest("Unknown object type"))
	}
}

pub async fn user() -> Result<String, ActixError> {
	// Maybe a web interface
	todo!()
}

pub async fn user_json(req: HttpRequest) -> impl Responder {
	let uri = req.uri();
	let uri_str = &uri.to_string();
	let mut uri_parts = Parts::from(uri.to_owned());
	let mut actor = actor::Group::default();
	uri_parts.path_and_query = Some(PathAndQuery::from_maybe_shared(
		String::from("/for/") + &req.match_info().query("actorname")
	).unwrap());

	actor.ap_actor_props.inbox = Uri::from_parts(uri_parts).unwrap().to_string().into();
	actor.ap_actor_props.outbox = (uri_str.clone() + "/all").into();
	actor.object_props.id = Some(Value::from(uri_str.to_owned()));
	actor.object_props.context = Some(Value::from(CONTEXT));
    serde_json::to_string(&actor)
}

pub async fn delivery() -> Result<String, ActixError> {
	todo!()
}
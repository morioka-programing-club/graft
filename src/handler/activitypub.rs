use mongodb::Database;
use mongodb::options::InsertOneOptions;
use actix_web::{web::{self, Path, Data}, HttpRequest, error::{self, Error as ActixError, ErrorInternalServerError as Error500}};
use actix_web::http::uri::{Uri, Parts, PathAndQuery};
use chrono::{Utc, DateTime};
use futures::future::TryFutureExt;
use serde_json::Value;
use std::sync::Arc;
use bson::{doc, to_bson, Bson, ordered::ValueAccessError};
use uuid::Uuid;

use activitystreams::collection::OrderedCollection;
use activitystreams::actor::{Person, Group};
use activitystreams::actor::properties::ApActorProperties as ApActorProps;
use activitystreams::object::properties::ObjectProperties as ObjectProps;
use activitystreams::collection::properties::CollectionPropertiesItemsEnum;

use crate::db::{process_senders, process_recievers, expect_single, get_object};
use crate::util::{is_username, Reference, is_string_numeric, to_document, rename_property};
use crate::model::Actor;

const PROTOCOL_HOST: &str = "https://localhost:8088";
const CONTEXT: &str = "http://www.w3.org/ns/activitystreams";

lazy_static! {
	static ref ID: String = "id".to_string();
}

pub async fn account(req: HttpRequest, params: Path<Reference>, db: Data<Database>) -> Result<String, ActixError> {
	let uri = req.uri();
	let uri_str = &uri.to_string();

	let mut actor: Actor = get_object(uri_str, db).await?;

	let mut uri_parts = Parts::from(uri.to_owned());
	uri_parts.path_and_query = Some(PathAndQuery::from_maybe_shared(String::from("/for/") + &params.id).unwrap());
	AsMut::<ApActorProps>::as_mut(&mut actor).set_inbox(Uri::from_parts(uri_parts).unwrap().to_string()).map_err(Error500)?;

	AsMut::<ApActorProps>::as_mut(&mut actor).outbox = (uri_str.clone() + "/all").parse().map_err(Error500)?;
	AsMut::<ObjectProps>::as_mut(&mut actor).id = Some(uri_str.parse().map_err(Error500)?);
	AsMut::<ObjectProps>::as_mut(&mut actor).set_context_xsd_any_uri(CONTEXT);
    serde_json::to_string(&actor).map_err(Error500)
}

pub async fn inbox(req: HttpRequest, db: Data<Database>) -> Result<String, ActixError> {
	let uri = req.uri().to_string();
	let mut inbox = OrderedCollection::default();
	AsMut::<ObjectProps>::as_mut(&mut inbox).id = Some(uri.parse().map_err(Error500)?);
	AsMut::<ObjectProps>::as_mut(&mut inbox).set_context_xsd_any_uri(CONTEXT);

	let mut items: CollectionPropertiesItemsEnum = get_object(&uri, db).await?;
	inbox.collection_props.items = items.into();
	Ok(serde_json::to_string(&inbox).map_err(|_| panic!("JSON serialization error"))?)
}

pub async fn outbox(req: HttpRequest, db: Data<Database>) -> Result<String, ActixError> {
	let uri = req.uri().to_string();
	let mut outbox = OrderedCollection::default();
	AsMut::<ObjectProps>::as_mut(&mut outbox).id = Some(uri.parse().map_err(Error500)?);
	AsMut::<ObjectProps>::as_mut(&mut outbox).set_context_xsd_any_uri(CONTEXT);

	let mut items: CollectionPropertiesItemsEnum = get_object(&uri, db).await?;
	outbox.collection_props.items = items.into();
	Ok(serde_json::to_string(&outbox).map_err(|_| panic!("JSON serialization error"))?)
}

pub async fn post(req: HttpRequest, db: Data<Database>) -> Result<String, ActixError> {
	if !is_string_numeric(req.match_info().query("id")) {return Err(error::ErrorBadRequest("Object ID is numeric"));}

	let items = get_object(&req.uri().to_string(), db).await?;
	Ok(serde_json::to_string(&items).map_err(|_| panic!("JSON serialization error"))?)
}

pub async fn get_changelog(req: HttpRequest, db: Data<Database>) -> Result<String, ActixError> {
	if !is_string_numeric(req.match_info().query("id")) {return Err(error::ErrorBadRequest("Object ID is numeric"));}

	let history = get_object(&req.uri().to_string(), db).await?;
	Ok(serde_json::to_string(&Value::Array(history)).map_err(|_| panic!("JSON serialization error"))?)
}

pub async fn create_account(req: HttpRequest, db: Data<Database>) -> Result<String, ActixError> {
	let mut name = req.match_info().query("actorname").to_owned();
	let isuser = is_username(&name);
	if isuser {name.remove(0);}
	let mut account = if isuser {
		Actor::Person(Person::new())
	} else {
		Actor::Group(Group::new())
	};
	AsMut::<ObjectProps>::as_mut(&mut account).id = Some(req.uri().to_string().parse().map_err(Error500)?);

	let mut account = to_document(to_bson(&account).map_err(Error500)?);
	rename_property(&mut account, "id", "_id".to_string());
	db.collection("objects").insert_one(account, InsertOneOptions::default());
	Ok(if isuser {"User"} else {"Group"}.to_owned() + " succesfully created")
}

pub async fn submit(req: HttpRequest, mut json: web::Json<Value>, db: Data<Database>) -> Result<String, ActixError> {
	if !json["type"].is_string() {return Err(error::ErrorBadRequest("Non-activitystream object recieved"))}

	match json["type"].as_str().unwrap() { // okay unwrap here, checked above
		"Create" => {
			db.collection("objects").insert_one(to_document(json.into()), InsertOneOptions::default());
			Ok("".to_string())
		},
		"Update" => {
			let message = get_object(&req.uri().to_string(), db).await?;

			let id = message.get_i64("_id").map_err(Error500)?;
			let logid = match message.get_i64("changelog") {
				Ok(id) => Ok(id),
				Err(error) => match error {
					ValueAccessError::NotPresent => {
						Ok(db.collection("objects").insert_one(doc!{
							"_id": to_bson(&Uuid::new_v4()).map_err(Error500)?, "versions": []
						}, InsertOneOptions::default()).await.map_err(Error500)?.inserted_id)
					},
					ValueAccessError::UnexpectedType => Err(ValueAccessError::UnexpectedType)
				}
			};
			db.collection("objects").insert_one(to_document(json.into()), InsertOneOptions::default());
			let old = expect_single("Internal error", "Internal error")(
				db.cquery(create_message, &[
					&message.get::<&str, &str>("content"), &message.get::<&str, DateTime<Utc>>("time"), &logid
				]).map_err(error::ErrorInternalServerError).await?)?;

			db.cquery(add_message_version, &[&logid, &old.get::<&str, i64>("id")])
				.map_err(error::ErrorInternalServerError).await?;
			db.cquery(update_message, &[&id, &json["object"]["content"].as_str(), &Utc::now(), &logid])
				.map_err(error::ErrorInternalServerError).await?;
			Ok("".to_string())
		},
		_ => Err(error::ErrorBadRequest("Unknown object type"))
	}
}

pub async fn delivery() -> Result<String, ActixError> {
	todo!()
}
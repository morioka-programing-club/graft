use actix_web::error::Error as ActixError;
use chrono::{DateTime, SecondsFormat, Utc};
use futures::{StreamExt, TryStreamExt};
use mongodb::bson::{self, doc, from_bson, to_bson, Bson, Document};
use mongodb::options::{FindOneOptions, FindOptions, InsertOneOptions};
use mongodb::results::InsertOneResult;
use mongodb::Client;
use serde_json::{Map, Value};

use crate::error::internal_error;
use crate::util::ObjectId;
use crate::DB_NAME;

pub async fn insert(doc: &Map<String, Value>, db: &Client) -> Result<InsertOneResult, ActixError> {
	db.database(&DB_NAME)
		.collection("objects")
		.insert_one(to_db_object(doc)?, InsertOneOptions::default())
		.await
		.map_err(internal_error)
}

async fn get_with_query(db: &Client, query: Document) -> Result<Option<Map<String, Value>>, ActixError> {
	db.database(&DB_NAME)
		.collection("objects")
		.find_one(
			query,
			FindOneOptions::builder().sort(Some(doc! { "_id.t": -1 })).build() // get latest
		)
		.await
		.map_err(internal_error)
		.map(|opt| opt.map(from_db_object).transpose())
		.flatten()
}

async fn get_all_with_query(db: &Client, query: Document) -> Result<Vec<Map<String, Value>>, ActixError> {
	db.database(&DB_NAME)
		.collection("objects")
		.find(
			query,
			FindOptions::builder().sort(Some(doc! { "_id.t": 1 })).build()
		)
		.await
		.map_err(internal_error)? // Error from find query
		.map_err(internal_error) // Error from returned stream
		.map_ok(|doc| from_db_object(doc))
		.map(|result| result.flatten())
		.try_collect().await
}

pub async fn get(id: &ObjectId, db: &Client) -> Result<Option<Map<String, Value>>, ActixError> {
	get_with_query(db, doc! { "_id.id": id }).await
}

pub async fn get_record(id: &ObjectId, time: &DateTime<Utc>, db: &Client) -> Result<Option<Map<String, Value>>, ActixError> {
	get_with_query(db, doc! { "_id.id": id, "_id.t": { "$lte": time } }).await
}

pub async fn get_replies(id: &ObjectId, db: &Client) -> Result<Vec<Map<String, Value>>, ActixError> {
	get_all_with_query(db, doc! { "inReplyTo": id }).await
}

pub async fn get_children(id: &ObjectId, db: &Client) -> Result<Vec<Map<String, Value>>, ActixError> {
	get_all_with_query(db, doc! { "context": id }).await
}

fn from_db_object(mut doc: Document) -> Result<Map<String, Value>, ActixError> {
	if let Bson::Document(mut _id) = doc.remove("_id").ok_or_else(|| internal_error("`_id` is missing"))? {
		doc.insert("id", _id.remove("id").ok_or_else(|| internal_error("`_id.id` is missing"))?);
		doc.insert("updated", _id.remove("t").ok_or_else(|| internal_error("`_id.t` is missing"))?);
	} else {
		return Err(internal_error("`_id` is not a document"));
	}
	let mut bson = Bson::Document(doc);
	traverse_bson(&mut bson, &mut |bson| match bson {
		Bson::ObjectId(oid) => *bson = Bson::String(oid.to_hex()),
		Bson::DateTime(time) => *bson = Bson::String(time.to_chrono().to_rfc3339_opts(SecondsFormat::Millis, true)),
		_ => ()
	});
	from_bson(bson).map_err(internal_error)
}

fn to_db_object(object: &Map<String, Value>) -> Result<Document, ActixError> {
	let mut bson = to_bson(object).map_err(internal_error)?;
	parse_bson_values(&mut bson);
	let mut doc = if let Bson::Document(doc) = bson { doc } else { unreachable!() };
	let id = doc.remove("id").ok_or_else(|| internal_error("`id` is missing"))?;
	let updated = doc.remove("updated").ok_or_else(|| internal_error("`updated` is missing"))?;
	doc.insert("_id", doc! { "id": id, "t": updated });
	Ok(doc)
}

fn parse_bson_values(bson: &mut Bson) {
	traverse_bson(bson, &mut |bson| {
		if let Bson::String(s) = bson {
			// Some plain strings may also gets coerced here, but they should be converted back anyway
			if let Ok(oid) = s.parse::<bson::oid::ObjectId>() {
				*bson = Bson::ObjectId(oid)
			} else if let Ok(time) = s.parse::<DateTime<Utc>>() {
				*bson = Bson::DateTime(time.into())
			}
		}
	});
}

fn traverse_bson(bson: &mut Bson, f: &mut impl FnMut(&mut Bson)) {
	f(bson);
	match bson {
		// Hack with slight performance loss. `&mut bson::Document` really needs an `IntoIterator` impl.
		Bson::Document(doc) => {
			for k in doc.keys().cloned().collect::<Vec<_>>() {
				traverse_bson(doc.get_mut(k).unwrap(), f);
			}
		}
		Bson::Array(array) => {
			for prop in array {
				traverse_bson(prop, f);
			}
		}
		_ => ()
	}
}

// fn from_oid(id: Bson) -> Option<String> {
// if let Bson::Binary(Binary { bytes: id, .. }) = id {
// Some(format!("{:x}", Bytes::from(id)))
// } else {
// None
// }
// }

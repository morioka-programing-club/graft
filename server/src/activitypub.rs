use std::convert::TryInto;

use actix_web::dev::RequestHead;
use actix_web::error::{Error, ErrorBadRequest, ErrorMethodNotAllowed};
use actix_web::http::Method;
use chrono::{DateTime, SecondsFormat, Utc};
use json_trait::ForeignMutableJson;
use mime::Mime;
use once_cell::sync::Lazy;
use serde_json::{json, Map, Value};

use crate::error::internal_error;
use crate::util::{ObjectId, Url};

macro_rules! ns {
	($ns:tt:$name:tt) => { concat!(ns!($ns:), stringify!($name)) };
	($ns:tt:) => { concat!(ns!($ns), "#") };
	(as) => { "https://www.w3.org/ns/activitystreams" };
	(ldp) => { "http://www.w3.org/ns/ldp" };
}

mod handler;
mod jsonld;
mod strip;

pub use handler::*;

static CONTEXT: Lazy<Value> = Lazy::new(|| Value::String(ns!(as).to_string()));
// Setting base URLs for existing properties is technically not conformant. This context is to be provided opt-in.
// If we ever host this somewhere, instead of replacing this with bare URL, context object with @import will be needed
// to preserve the relative URL semantics.
static GRAFT_CONTEXT: Lazy<Value> = Lazy::new(|| {
	json!({ // Compact ids to bare oids
		ns!(as:actor): {
			"@context": {
				"@base": "../of/"
			}
		},
		ns!(as:attributedTo): {
			"@context": {
				"@base": "../of/"
			}
		},
		ns!(ldp:inbox): {
			"@context": {
				"@base": "../for/"
			}
		},
		ns!(as:outbox): {
			"@context": {
				"@base": "../by/"
			}
		},
		ns!(as:object): {
			"@context": {
				"@base": "../post/"
			}
		}
	})
});

// Too bad macros can't easily do this.
#[non_exhaustive]
#[derive(serde::Deserialize, Debug, PartialEq, Eq)]
enum SupportedActivity {
	#[serde(rename = "https://www.w3.org/ns/activitystreams#Create")]
	Create,
	#[serde(rename = "https://www.w3.org/ns/activitystreams#Update")]
	Update,
	#[serde(rename = "https://www.w3.org/ns/activitystreams#Delete")]
	Delete,
	#[serde(rename = "https://www.w3.org/ns/activitystreams#Follow")]
	Follow,
	#[serde(rename = "https://www.w3.org/ns/activitystreams#Add")]
	Add,
	#[serde(rename = "https://www.w3.org/ns/activitystreams#Remove")]
	Remove,
	#[serde(rename = "https://www.w3.org/ns/activitystreams#Like")]
	Like,
	#[serde(rename = "https://www.w3.org/ns/activitystreams#Block")]
	Block,
	#[serde(rename = "https://www.w3.org/ns/activitystreams#Undo")]
	Undo
}

fn get_id(object: &Map<String, Value>) -> Result<ObjectId, Error> {
	object
		.get("@id")
		.ok_or(ErrorBadRequest("`id` is missing"))?
		.as_str()
		.expect("valid JSON-LD")
		.parse::<Url>()
		.map_err(internal_error)?
		.try_into()
		.map_err(ErrorBadRequest)
}

fn is_collection(object: &Map<String, Value>) -> Result<bool, Error> {
	Ok(object
		.get("@type")
		.ok_or(ErrorBadRequest("missing `type`"))?
		.as_array()
		.expect("expanded object")
		.iter()
		.any(|ty| match ty.as_str() {
			Some("Collection" | "OrderedCollection") => true,
			_ => false
		}))
}

fn get_objects<'a>(object: &'a Map<String, Value>, prop: &str) -> Option<impl Iterator<Item = &'a Map<String, Value>>> {
	object
		.get(prop)
		.map(|array| array.as_array().expect("expanded object").iter().map(|inner| inner.as_object().expect("expanded object")))
}

fn get_objects_mut<'a>(object: &'a mut Map<String, Value>, prop: &str) -> Option<impl Iterator<Item = &'a mut Map<String, Value>>> {
	object.get_mut(prop).map(|array| {
		array
			.as_array_mut()
			.expect("expanded object")
			.iter_mut()
			.map(|inner| inner.as_object_mut().expect("expanded object"))
	})
}

fn take_objects(object: &mut Map<String, Value>, prop: &str) -> Option<impl Iterator<Item = Map<String, Value>>> {
	object.remove(prop).map(|array| {
		array
			.into_array()
			.expect("expanded object")
			.into_iter()
			.map(|inner| inner.into_object().expect("expanded object"))
	})
}

fn copy_recipients(from: &Map<String, Value>, to: &mut Map<String, Value>) {
	for key in ["to", "cc", "bto", "bcc", "audience"].iter().filter(|key| from.contains_key(**key)) {
		if to.contains_key(*key) {
			let to = to[*key].as_array_mut().expect("expanded value");
			for from in from[*key].as_array().expect("expanded value") {
				if !to.contains(from) {
					to.push(from.clone());
				}
			}
		} else {
			to.insert(key.to_string(), from[*key].clone());
		}
	}
}

// pub fn get_url_decoration(object: &Map<String, Value>) -> Result<String, Error> {
// if let Some(Value::String(ref name)) = object.get("name") {
// if name.contains(&['/', '-'] as &[_]) { return Err(ErrorBadRequest("slash and hyphen are not allowed in user name")); }
// Ok(name.to_string() + "-")
// } else {
// Ok("".to_string())
// }
// }

fn get_request_type(head: &RequestHead) -> Result<Vec<Mime>, Error> {
	let key = match head.method {
		Method::GET => "Accept",
		Method::POST => "Content-Type",
		_ => return Err(ErrorMethodNotAllowed(""))
	};
	head.headers.get(key).map_or(Ok(vec![]), |ty| {
		ty.to_str().map_err(ErrorBadRequest)?.split(',').map(|mime| mime.parse().map_err(ErrorBadRequest)).collect()
	})
}

pub fn is_activitypub_request(head: &RequestHead) -> bool {
	if let Ok(mimes) = get_request_type(head) {
		for mime in mimes {
			match mime.essence_str() {
				"application/activity+json" => return true,
				"application/ld+json" if mime.get_param("profile").map_or(false, |profile| profile.as_str().split(' ').any(|iri| iri == ns!(as))) => return true,
				_ => ()
			}
		}
	}
	false
}

fn datetime(time: DateTime<Utc>) -> Value {
	json!({ "@type": "xsd:dateTime" , "@value": time.to_rfc3339_opts(SecondsFormat::Millis, true) })
}

use std::convert::TryInto;

use actix_web::{http::Method, dev::RequestHead};
use actix_web::error::{Error, ErrorBadRequest, ErrorMethodNotAllowed};
use chrono::{DateTime, Utc, SecondsFormat};
use mime::Mime;
use serde_json::{Map, Value, json};

use crate::util::{ObjectId, Url};
use crate::error::internal_error;

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

lazy_static::lazy_static! {
	static ref CONTEXT: Value = Value::String(ns!(as).to_string());
	// Setting base URLs for existing properties is technically not conformant. This context is to be provided opt-in.
	// If we ever host this somewhere, instead of replacing this with bare URL, context object with @import will be needed
	// to preserve the relative URL semantics.
	static ref GRAFT_CONTEXT: Value = json!({ // Compact ids to bare oids
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
		}
	});
}

// Too bad macros can't easily do this.
#[non_exhaustive]
#[derive(serde::Deserialize, Debug, PartialEq, Eq)]
enum SupportedActivity {
	#[serde(rename = "https://www.w3.org/ns/activitystreams#Create")] Create,
	#[serde(rename = "https://www.w3.org/ns/activitystreams#Update")] Update,
	#[serde(rename = "https://www.w3.org/ns/activitystreams#Delete")] Delete
}

fn get_id(object: &Map<String, Value>) -> Result<ObjectId, Error> {
	object.get("@id").ok_or(ErrorBadRequest("`id` is missing"))?
		.as_str().expect("valid JSON-LD")
   		.parse::<Url>().map_err(internal_error)?
		.try_into().map_err(ErrorBadRequest)
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

/*pub fn get_url_decoration(object: &Map<String, Value>) -> Result<String, Error> {
	if let Some(Value::String(ref name)) = object.get("name") {
		if name.contains(&['/', '-'] as &[_]) { return Err(ErrorBadRequest("slash and hyphen are not allowed in user name")); }
		Ok(name.to_string() + "-")
	} else {
		Ok("".to_string())
	}
}*/

fn get_request_type(head: &RequestHead) -> Result<Option<Mime>, Error> {
	let key = match head.method {
		Method::GET => "Accept",
		Method::POST => "Content-Type",
		_ => return Err(ErrorMethodNotAllowed(""))
	};
	head.headers.get(key).map(|ty| ty.to_str().map_err(ErrorBadRequest)?.parse().map_err(ErrorBadRequest)).transpose()
}

pub fn is_activitypub_request(head: &RequestHead) -> bool {
	if let Ok(Some(mime)) = get_request_type(head) {
		match mime.essence_str() {
			"application/activity+json" => true,
			"application/ld+json" => mime.get_param("profile")
				.map_or(false, |profile| profile.as_str().split(' ').any(|iri| iri == ns!(as))),
			_ => false
		}
	} else {
		false
	}
}

fn datetime(time: DateTime<Utc>) -> Value {
	json!({ "@type": "xsd:dateTime" , "@value": time.to_rfc3339_opts(SecondsFormat::Millis, true) })
}
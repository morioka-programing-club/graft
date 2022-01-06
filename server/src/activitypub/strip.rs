use json_ld_rs::error::JsonLdError;
use json_ld_rs::JsonLdOptions;
use serde_json::{json, Map, Value};

use super::jsonld::*;
use super::{CONTEXT, GRAFT_CONTEXT};
use crate::util::get_oid;

pub async fn unstrip_object(mut object: Map<String, Value>, options: &JsonLdOptions<'_, Value>) -> Result<Map<String, Value>, JsonLdError> {
	match object.remove("@context") {
		Some(Value::Array(mut ctx)) => {
			insert_graft_context(&mut ctx);
			insert_context(&mut ctx);

			object.insert("@context".to_string(), Value::Array(ctx));
		}
		Some(ctx) if ctx == *GRAFT_CONTEXT || ctx == *CONTEXT => {
			object.insert("@context".to_string(), json!([CONTEXT.clone(), GRAFT_CONTEXT.clone()]));
		}
		None => {
			object.insert("@context".to_string(), json!([CONTEXT.clone(), GRAFT_CONTEXT.clone()]));
		}
		Some(ctx) => {
			object.insert("@context".to_string(), json!([CONTEXT.clone(), GRAFT_CONTEXT.clone(), ctx]));
		}
	}
	expand_object(&object, options).await
}

pub async fn unstrip_actor(object: Map<String, Value>, options: &JsonLdOptions<'_, Value>) -> Result<Map<String, Value>, JsonLdError> {
	let mut object = unstrip_object(object, options).await?;
	if let Some(oid) = get_oid(object.get("@id").and_then(|id| id.as_str()).expect("Objects must have an id")).map(|oid| oid.to_string()) {
		if !object.contains_key("inbox") {
			object.insert("inbox".to_string(), ("../for/".to_string() + &oid).into());
		}
		if !object.contains_key("outbox") {
			object.insert("outbox".to_string(), ("../by/".to_string() + &oid).into());
		}
	}
	Ok(object)
}

pub async fn strip_object(object: &Map<String, Value>, mut context: Vec<Value>, options: &JsonLdOptions<'_, Value>) -> Result<Map<String, Value>, JsonLdError> {
	insert_graft_context(&mut context);
	let mut object = compact_object(object, context, options).await?;
	match object.remove("@context") {
		Some(Value::String(ctx)) if ctx == ns!(as) => {}
		Some(Value::Array(ctx)) => {
			let mut ctx = ctx.into_iter().peekable();
			if ctx.peek() == Some(&*CONTEXT) {
				ctx.next();
			}
			if ctx.peek() == Some(&*GRAFT_CONTEXT) {
				ctx.next();
			}
			if let Some(_) = ctx.peek() {
				object.insert("@context".to_string(), ctx.collect::<Vec<_>>().into());
			}
		}
		Some(ctx) => {
			object.insert("@context".to_string(), ctx);
		}
		None => {}
	}
	if let Some("") = object.get("inbox").and_then(|inbox| inbox.as_str()).as_deref() {
		object.remove("inbox");
	}
	if let Some("") = object.get("outbox").and_then(|outbox| outbox.as_str()).as_deref() {
		object.remove("outbox");
	}
	Ok(object)
}

#[cfg(test)]
mod tests {
	use super::*;
	use json_ld_rs::JsonOrReference;
	use json_trait::ForeignMutableJson;
	use std::borrow::Cow;

	#[actix_web::test]
	async fn test_strip() {
		let object = json!({
		  "@id": "random-id",
		  "@type": "Note",
		  ns!(as:attributedTo): {"@id": "../of/some-actor"},
		  ns!(as:published): {"@type": "xsd:dateTime", "@value": "2021-12-22T18:29:00Z"},
		});
		let options = JsonLdOptions {
			base: Some("https://example.org/post/random-id".to_string()),
			expand_context: Some(JsonOrReference::Reference(Cow::Borrowed(ns!(as)))),
			..JsonLdOptions::default()
		};
		let result = strip_object(object.as_object().unwrap(), vec![json!(ns!(as))], &options).await.unwrap();
		assert_eq!(
			Value::Object(result),
			json!({
			  "id": "random-id",
			  "type": "Note",
			  "attributedTo": "some-actor",
			  "published": "2021-12-22T18:29:00Z",
			})
		);
	}

	#[actix_web::test]
	async fn test_unstrip() {
		let object = json!({
		  "id": "random-id",
		  "type": "Note",
		  "attributedTo": "some-actor",
		  "published": "2021-12-22T18:29:00Z",
		});
		let options = JsonLdOptions {
			base: Some("https://example.org/post/random-id".to_string()),
			expand_context: Some(JsonOrReference::Reference(Cow::Borrowed(ns!(as)))),
			..JsonLdOptions::default()
		};
		let result = unstrip_object(object.into_object().unwrap(), &options).await.unwrap();
		assert_eq!(
			Value::Object(result),
			json!({
			  "@id": "https://example.org/post/random-id",
			  "@type": [ns!(as:Note)],
			  ns!(as:attributedTo): [{"@id": "https://example.org/of/some-actor"}],
			  ns!(as:published): [{"@type": "http://www.w3.org/2001/XMLSchema#dateTime", "@value": "2021-12-22T18:29:00Z"}],
			})
		);
	}
}

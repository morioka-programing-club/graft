use serde_json::{Map, Value};
use json_ld_rs::JsonLdOptions;
use json_ld_rs::error::JsonLdError;

use super::{CONTEXT, GRAFT_CONTEXT};
use super::jsonld::*;
use crate::util::get_oid;

pub async fn unstrip_object(object: Map<String, Value>, options: &JsonLdOptions<'_, Value>) -> Result<Map<String, Value>, JsonLdError> {
	expand_object(object, options).await
}

pub async fn unstrip_actor(mut object: Map<String, Value>, options: &JsonLdOptions<'_, Value>) -> Result<Map<String, Value>, JsonLdError> {
	object = unstrip_object(object, options).await?;
	if let Some(oid) = get_oid(object.get("@id").and_then(|id| id.as_str()).expect("Objects must have an id")).map(|oid| oid.to_string()) {
		if !object.contains_key("inbox") { object.insert("inbox".to_string(), ("../for/".to_string() + &oid).into()); }
		if !object.contains_key("outbox") { object.insert("outbox".to_string(), ("../by/".to_string() + &oid).into()); }
	}
	Ok(object)
}

pub async fn strip_object(mut object: Map<String, Value>, mut context: Vec<Value>, options: &JsonLdOptions<'_, Value>) ->
	Result<Map<String, Value>, JsonLdError>
{
	prepend_graft_context(&mut context);
	object = compact_object(object, context, options).await?;
	match object.remove("@context") {
		Some(Value::String(ctx)) if ctx == ns!(as) => {},
		Some(Value::Array(ctx)) => {
			let mut ctx = ctx.into_iter().peekable();
			if ctx.peek() == Some(&*GRAFT_CONTEXT) { ctx.next(); }
			if ctx.peek() == Some(&*CONTEXT) { ctx.next(); }
			if let Some(_) = ctx.peek() { object.insert("@context".to_string(), ctx.collect::<Vec<_>>().into()); }
		},
		Some(ctx) => {
			object.insert("@context".to_string(), ctx);
		},
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
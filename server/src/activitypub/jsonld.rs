use std::borrow::Cow;
use std::convert::TryFrom;

use actix_web::{HttpRequest, dev::RequestHead};
use actix_web::error::Error as ActixError;
use serde_json::{Map, Value};
use json_ld_rs::{compact, expand, error::JsonLdError, JsonLdInput, JsonLdOptions, JsonOrReference};

use super::{CONTEXT, GRAFT_CONTEXT, get_request_type};
use crate::util::Url;

pub fn json_ld_options(req: &HttpRequest) -> Result<JsonLdOptions<'static, Value>, ActixError> {
	Ok(JsonLdOptions {
		base: Some(Url::try_from(req)?.to_string()),
		expand_context: Some(JsonOrReference::Reference(Cow::Borrowed(ns!(as)))),
		..JsonLdOptions::default()
	})
}

pub fn prepend_graft_context(context: &mut Vec<Value>) {
	if context.iter().all(|ctx| ctx != &*GRAFT_CONTEXT) { context.insert(0, GRAFT_CONTEXT.clone()); }
}

pub fn prepend_context(context: &mut Vec<Value>) {
	if context.iter().all(|ctx| ctx != &*CONTEXT) { context.insert(0, CONTEXT.clone()); }
}

pub fn context(object: &Map<String, Value>, head: &RequestHead) -> Result<Vec<Value>, ActixError> {
	let mut ctx = match object.get("@context").cloned() {
		Some(Value::Array(ctx)) => ctx,
		Some(ctx) => vec![ctx],
		None => vec![]
	};
	for mime in get_request_type(head)? {
		if mime.get_param("profile").map_or(false, |profile| profile.as_str().split(' ').any(|iri| false)) {
			prepend_graft_context(&mut ctx);
			break;
		}
	}
	prepend_context(&mut ctx);
	Ok(ctx)
}

pub async fn expand_object(object: &Map<String, Value>, options: &JsonLdOptions<'_, Value>) -> Result<Map<String, Value>, JsonLdError> {
	if let Value::Object(object) = expand(JsonLdInput::<Value>::JsonObject(object), options).await?.remove(0) {
		Ok(object)
	} else {
		panic!()
	}
}

pub async fn compact_object(object: &Map<String, Value>, mut context: Vec<Value>, options: &JsonLdOptions<'_, Value>) ->
	Result<Map<String, Value>, JsonLdError>
{
	let mut ctx_index = None;
	for (i, ctx) in context.iter_mut().enumerate() {
		if ctx == &*GRAFT_CONTEXT {
			if let Some(ref base) = options.base {
				ctx_index = Some(i);
				ctx.as_object_mut().unwrap().insert("@base".to_string(), base.clone().into());
			}
		}
	}
	let mut result = compact(JsonLdInput::<Value>::JsonObject(object), Some(Cow::Owned(Value::Array(context))), options)
		.await?;
	if let Some(i) = ctx_index {
		result["@context"][i].as_object_mut().unwrap().remove("@base");
	}
	Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
	use json_trait::{BuildableJson, json};
    use actix_web::test::TestRequest;

    #[test]
    fn test_context() {
        let req = TestRequest::get()
			.append_header(("accept", r#"application/ld+json; profile="https://www.w3.org/ns/activitystreams""#))
			.to_http_request();
        assert_eq!(context(&json!(Value, {}), req.head()).unwrap(), vec![json!(Value, ns!(as))]);
		assert_eq!(context(&json!(Value, { "@context": ns!(as) }), req.head()).unwrap(), vec![json!(Value, ns!(as))]);
		assert_eq!(context(&json!(Value, { "@context": [ ns!(as) ] }), req.head()).unwrap(), vec![json!(Value, ns!(as))]);
		assert_eq!(
			context(&json!(Value, { "@context": "https://example.org/ns/" }), req.head()).unwrap(),
			vec![json!(Value, ns!(as)), json!(Value, "https://example.org/ns/")]
		);
    }
}
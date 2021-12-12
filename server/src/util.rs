use std::future::{Future, ready, Ready};
use std::str::FromStr;
use std::convert::{TryFrom, TryInto};
use std::ops::Deref;

use serde::Deserialize;
use serde_json::Value;
use mongodb::bson::{self, Bson};
use actix_web::{HttpRequest, error, FromRequest, dev::Payload};
use url::Url as NativeUrl;

// This needs to be a function in order for inference to work
pub async fn call_handler<F, T>(handler: F, req: &HttpRequest) -> Result<<F::Output as Future>::Output, T::Error>
where
	F: Fn<T>,
	F::Output: Future,
	T: FromRequest
{
	Ok(handler.call(T::extract(req).await?).await)
}

pub struct Url(pub NativeUrl);

impl TryFrom<&HttpRequest> for Url {
	type Error = error::Error;
	fn try_from(req: &HttpRequest) -> Result<Self, Self::Error> {
		let path = req.path();
		let url = if path.starts_with('/') {
			let conn = req.connection_info();
			(&format!(
				"{}://{}{}",
				conn.scheme(),
				conn.host(),
				path
			)).parse()
		} else {
			path.parse()
		};
		url.map_err(crate::error::internal_error)
	}
}

impl FromStr for Url {
	type Err = url::ParseError;
	fn from_str(s: &str) -> Result<Self, Self::Err> {
		NativeUrl::parse(s).map(Url)
	}
}

impl Deref for Url {
	type Target = NativeUrl;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl FromRequest for Url {
	type Error = error::Error;
	type Future = Ready<Result<Self, Self::Error>>;

	fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
		ready(req.try_into())
	}
}

/*pub fn url_for(req: &HttpRequest) -> Result<NativeUrl, error::Error> {
	let id = req.match_info().get("id").ok_or(error::ErrorInternalServerError("Internal Server Error"))?;
	url_for_id(req, id)
}*/

#[derive(Debug, Deserialize)]
pub struct ObjectId(bson::oid::ObjectId);

impl ToString for ObjectId {
	fn to_string(&self) -> String {
		//self.0.to_simple().encode_lower(&mut [0u8; 32]).to_string()
		self.0.to_hex()
	}
}

impl TryFrom<Url> for ObjectId {
	type Error = error::Error;

	fn try_from(url: Url) -> Result<Self, Self::Error> {
		if url.host_str() != Some(crate::HOST) {
			return Err(error::ErrorBadRequest("Found bad URL"));
		}
		let path = url.path();
		if let Some(i) = path.rfind('-').or(path.rfind('/')) {
			path[(i + 1)..].parse().map_err(error::ErrorBadRequest)
		} else {
			Err(error::ErrorBadRequest("Found bad URL"))
		}
	}
}

impl FromStr for ObjectId {
	type Err = bson::oid::Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		s.parse().map(ObjectId)
	}
}

impl FromRequest for ObjectId {
	type Error = error::Error;
	type Future = Ready<Result<Self, Self::Error>>;

	fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
		ready(req.match_info().get("id").expect("can't be used on routes without id in uri").parse().map_err(error::ErrorBadRequest))
	}
}

impl From<&ObjectId> for Bson {
	fn from(id: &ObjectId) -> Self {
		//Bson::Binary(Binary { subtype: mongodb::bson::spec::BinarySubtype::Uuid, bytes: (id.0.as_bytes() as &[u8]).into() })
		Bson::ObjectId(id.0)
	}
}

impl From<&ObjectId> for Value {
	fn from(id: &ObjectId) -> Self {
		id.to_string().into()
	}
}

pub fn get_oid(url: &str) -> Option<&str> {
	url.rfind('-').or(url.rfind('/')).map(|i| &url[(i + 1)..])
}

pub fn generate_id() -> ObjectId {
	//ObjectId(Uuid::new_v4())
	ObjectId(bson::oid::ObjectId::new())
}
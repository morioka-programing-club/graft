use actix_web::{HttpRequest, Responder, error::{Error as ActixError}};

use crate::activitypub;
use crate::util::call_handler;
use super::render;

pub async fn account() -> Result<String, ActixError> {
	todo!()
}

pub async fn post(req: HttpRequest) -> Result<impl Responder, ActixError> {
	render("Main", call_handler(activitypub::post, &req).await??.into_inner()).await
}

pub async fn record() -> Result<String, ActixError> {
	todo!()
}

pub async fn mentions() -> Result<String, ActixError> {
	todo!()
}


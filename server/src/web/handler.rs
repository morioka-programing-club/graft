use actix_web::error::Error as ActixError;
use actix_web::{HttpRequest, Responder};

use super::render;
use crate::activitypub;
use crate::util::call_handler;

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

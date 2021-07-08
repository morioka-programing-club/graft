use std::fmt::{Debug, Display};

use actix_web::HttpResponse;
use actix_web::error::{Error, InternalError};

pub fn internal_error(err: impl Debug + Display + 'static) -> Error {
	log::error!("{}", err);
	InternalError::from_response(err, HttpResponse::InternalServerError().finish()).into()
}
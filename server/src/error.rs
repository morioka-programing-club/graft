use std::fmt::{Debug, Display};

use actix_web::error::{Error, InternalError};
use actix_web::HttpResponse;

pub fn internal_error(err: impl Debug + Display + 'static) -> Error {
	log::error!("{}", err);
	InternalError::from_response(err, HttpResponse::InternalServerError().finish()).into()
}

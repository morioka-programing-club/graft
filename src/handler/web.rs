use actix_web::{HttpRequest, error::{Error as ActixError}};

// Maybe a web interface
pub async fn user() -> Result<String, ActixError> {
	todo!()
}

pub async fn group() -> Result<String, ActixError> {
	todo!()
}

pub async fn post(req: HttpRequest) -> Result<String, ActixError> {
	todo!()
}

pub async fn mentions() -> Result<String, ActixError> {
	todo!()
}
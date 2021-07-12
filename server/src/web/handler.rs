use actix_web::{HttpRequest, error::{Error as ActixError}};

// Maybe a web interface
pub async fn account() -> Result<String, ActixError> {
	todo!()
}

pub async fn post(req: HttpRequest) -> Result<String, ActixError> {
	todo!()
}

pub async fn record() -> Result<String, ActixError> {
	todo!()
}

pub async fn mentions() -> Result<String, ActixError> {
	todo!()
}


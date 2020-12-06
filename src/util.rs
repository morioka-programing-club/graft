use serde::Deserialize;
use bson::{Bson, Document};
use actix_web::error;

pub fn is_username(name: &str) -> bool {
	name.starts_with("@")
}

#[derive(Deserialize)]
pub struct Reference {
	pub id: String,
	pub url_decoration: String
}

pub fn is_string_numeric(input: &str) -> bool {
    for c in input.chars() {
        if !c.is_numeric() {return false;}
    }
    return true;
}

pub fn to_document(input: Bson) -> Document {
	if let Bson::Document(document) = input {document} else {panic!()}
}

pub fn rename_property(doc: &mut Document, oldkey: &str, newkey: String) -> Result<Bson, error::Error> {
	let value = doc.remove(oldkey).ok_or(error::ErrorInternalServerError(
		String::from("The object doesn't have the expected property ") + oldkey))?;
	if doc.contains_key(newkey) { return Err(error::ErrorInternalServerError("Key \"" + newkey + "\" is occupied")); }
	doc.insert_bson(newkey, value).unwrap_none();
	Ok(value)
}
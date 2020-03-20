use std::fmt::Display;
use actix_web::dev::RequestHead;
use chrono::{DateTime, TimeZone};
use chrono::format::{Item, Numeric, Pad, Fixed};
use serde_json::{Map, Value};
use tokio_postgres::Row;

use crate::db;

fn is_activitypub_header(head: &RequestHead, key: &str) -> bool {
	match head.headers.get(key) {
		Some(v) => v == "application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\""
			|| v == "application/activity+json",
		None => false
	}
}

pub fn is_activitypub_post(head: &RequestHead) -> bool {
	is_activitypub_header(head, "Content-Type")
}

pub fn is_activitypub_request(head: &RequestHead) -> bool {
	is_activitypub_header(head, "Accept")
}

// copied from https://docs.rs/chrono/0.4.7/src/chrono/format/mod.rs.html#260-263
macro_rules! lit  { ($x:expr) => (Item::Literal($x)) }
macro_rules! num0 { ($x:ident) => (Item::Numeric(Numeric::$x, Pad::Zero)) }

pub fn format_timestamp_rfc3339_seconds_omitted<T>(time: DateTime<T>) -> String
	where T: TimeZone,
		   T::Offset: Display
{
	time.format_with_items(vec![
		num0!(Year), lit!("-"), num0!(Month), lit!("-"), num0!(Day), lit!("T"), num0!(Hour), lit!(":"), num0!(Minute), Item::Fixed(Fixed::TimezoneOffsetColonZ)
	].into_iter()).to_string()
}

pub enum MaybeUnwrapped<T> {
	Single(T),
	Multiple(Vec<T>),
	None
}

impl Into<Value> for MaybeUnwrapped<Value> {
	fn into(self: Self) -> Value {
		match self {
			MaybeUnwrapped::Single(value) => value,
			MaybeUnwrapped::Multiple(vec) => Value::Array(vec),
			MaybeUnwrapped::None => Value::Null
		}
	}
}

pub fn unwrap_short_vec<T>(mut vec: Vec<T>) -> MaybeUnwrapped<T> {
	match vec.len() {
		0 => MaybeUnwrapped::None,
		1 => MaybeUnwrapped::Single(vec.remove(0)),
		_ => MaybeUnwrapped::Multiple(vec)
	}
}

pub fn message_to_json(row: Row) -> Map<String, Value> {
	row.columns().into_iter()
		.map(|col| {
			let name = col.name();
			(String::from(name), db::into_value(&row, &name, col.type_()))
		})
		.collect::<Map<String, Value>>()
}

#[cfg(test)]
mod tests {
	use super::{format_timestamp_rfc3339_seconds_omitted, unwrap_short_vec, MaybeUnwrapped};
	use chrono::{Utc, TimeZone};

    #[test]
    fn timestamp_formatting() {
		let time = Utc.ymd(2019, 9, 20).and_hms(11, 5, 34);
        assert_eq!(format_timestamp_rfc3339_seconds_omitted(time), "2019-09-20T11:05Z");
    }

	#[test]
    fn short_vec() {
		match unwrap_short_vec(vec![14]) {
			MaybeUnwrapped::Single(value) => assert_eq!(value, 14),
			_ => panic!("The Vec have single value on it and this code should not be reachable")
		}
		match unwrap_short_vec(vec![13, 21]) {
			MaybeUnwrapped::Multiple(value) => assert_eq!(value, vec![13, 21]),
			_ => panic!("The Vec have multiple values on it and this code should not be reachable")
		}
		assert!(if let MaybeUnwrapped::None = unwrap_short_vec::<i32>(vec![]) {true} else {false});
    }
}
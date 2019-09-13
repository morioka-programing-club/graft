use std::fmt::Display;
use actix_web::dev::RequestHead;
use chrono::{DateTime, TimeZone};
use chrono::format::{Item, Numeric, Pad, Fixed};

pub fn is_activitypub_request(head: &RequestHead) -> bool {
	match head.headers.get("Content-Type") {
		Some(v) => v == "application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\""
			|| v == "application/activity+json",
		None => false
	}
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
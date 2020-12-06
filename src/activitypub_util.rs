use std::fmt::Display;
use actix_web::dev::RequestHead;
use chrono::{DateTime, TimeZone};
use chrono::format::{Item, Numeric, Pad, Fixed};

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

#[cfg(test)]
mod tests {
	use super::format_timestamp_rfc3339_seconds_omitted;
	use chrono::{Utc, TimeZone};

    #[test]
    fn timestamp_formatting() {
		let time = Utc.ymd(2019, 9, 20).and_hms(11, 5, 34);
        assert_eq!(format_timestamp_rfc3339_seconds_omitted(time), "2019-09-20T11:05Z");
    }
}
use actix_web::dev::RequestHead;

pub fn is_activitypub_request(head: &RequestHead) -> bool {
	match head.headers.get("Content-Type") {
		Some(v) => v == "application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\""
			|| v == "application/activity+json",
		None => false
	}
}

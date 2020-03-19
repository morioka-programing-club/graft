use actix_web::{HttpServer, App, web::{resource, scope, get, post}, guard, middleware::Compress};
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use env_logger;
use dotenv;

mod db;
mod handler;
mod util;

mod activitypub_util;
use activitypub_util::is_activitypub_request;

#[macro_use]
extern crate lazy_static;

const HOST: &str = "localhost:8088";

#[actix_rt::main]
async fn main() {
	dotenv::from_filename("GRAFTCONFIG").ok();
	env_logger::init();

    // load ssl keys
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
	loop {
    	match builder.set_private_key_file("key.pem", SslFiletype::PEM) {
			Ok(_) => break,
			Err(_) => eprintln!("An error happened while verifying certification. If it was a typo in the passphrase, try again.")
		}
	}
    builder.set_certificate_chain_file("cert.pem").unwrap();

	HttpServer::new(|| {
		/*
		 * About actix-web
		 * scopes concat before resource URL
		 * use {<name>[:regex]} syntax to match into various strings
		 */

		App::new()
			.wrap(Compress::default())
			.service(scope("/of/{actorname:@?[^/]+}")
				.route("", post().to(handler::create))
				.service(resource("/all")
					.route(get().guard(is_activitypub_request).to(handler::outbox))
					.route(post().guard(is_activitypub_request).to(handler::submit))
				)
			).service(resource("/of/{actorname:[^/@][^/]*}")
				.route(get().guard(guard::Not(is_activitypub_request)).to(handler::group))
				.route(get().guard(is_activitypub_request).to(handler::group_json))
			).service(resource("/of/{actorname:@[^/]+}")
				.route(get().guard(guard::Not(is_activitypub_request)).to(handler::user))
				.route(get().guard(is_activitypub_request).to(handler::user_json))
			).route("/post/{id}", get().to(handler::view_post))
			.route("/log/{id}", get().to(handler::changelog))
			.service(resource("/for/{actorname}")
				.route(get().guard(is_activitypub_request).to(handler::inbox))
				.route(post().to(handler::delivery))
			)
	}).bind_openssl(HOST, builder).unwrap().run().await.unwrap();
}
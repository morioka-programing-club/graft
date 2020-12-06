use actix_web::{HttpServer, App, web::{resource, scope, get, post}, guard, middleware::Compress};
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use env_logger;
use dotenv;
use std::env::var as env;

mod db;
mod handler;
mod util;
mod activitypub_util;
mod model;

use activitypub_util::{is_activitypub_request, is_activitypub_post};

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
    	match builder.set_private_key_file(env("KEY").unwrap(), SslFiletype::PEM) {
			Ok(_) => break,
			Err(_) => eprintln!("An error happened while verifying certification. If it was a typo in the passphrase, try again.")
		}
	}
    builder.set_certificate_chain_file(env("CERT").unwrap()).unwrap();

	let db = mongodb::Client::with_uri_str(&env("CLUSTER_URI").unwrap()).await.unwrap().database(&env("DB_NAME").unwrap());

	HttpServer::new(|| {
		/*
		 * About actix-web
		 * scopes concat before resource URL
		 * use {<name>[:regex]} syntax to match into various strings
		 */

		App::new()
			.wrap(Compress::default())
			.data(db)
			.service(scope("/of/{id:@?[^-/]+}{url_decoration:(-.+)?}")
				.route("", post().to(handler::activitypub::create_account))
				.service(resource("/all")
					.route(get().guard(is_activitypub_request).to(handler::activitypub::outbox))
					.route(post().guard(is_activitypub_post).to(handler::activitypub::submit))
				)
			).service(resource("/of/{id:[^-/@][^-/]*}{url_decoration:(-.+)?}")
				.route(get().guard(guard::Not(is_activitypub_request)).to(handler::web::group))
			).service(resource("/of/{id:@[^-/]+}{url_decoration:(-.+)?}")
				.route(get().guard(guard::Not(is_activitypub_request)).to(handler::web::user))
			).service(resource("/of/{id:[^-/]+}{url_decoration:(-.+)?}")
				.route(get().guard(is_activitypub_request).to(handler::activitypub::account))
			).service(resource("/post/{id:[^-/]+}{url_decoration:(-.+)?}")
				.route(get().guard(guard::Not(is_activitypub_request)).to(handler::web::post))
				.route(get().guard(is_activitypub_request).to(handler::activitypub::post))
			)
			// There shouldn't be any situation where the user wants link to list of revisions.
			// As such, no HTML serving handler or URL decoration is implemented for changelogs.
			// Web client must use Javascript(and/or WebAssembly) to fetch them via ActivityPub interface and show.
			.route("/log/{id}", get().guard(is_activitypub_request).to(handler::activitypub::get_changelog))
			.service(resource("/for/{id:[^-/]+}{url_decoration:(-.+)?}")
				.route(get().guard(guard::Not(is_activitypub_request)).to(handler::web::mentions))
				.route(get().guard(is_activitypub_request).to(handler::activitypub::inbox))
				.route(post().to(handler::activitypub::delivery))
			)
	}).bind_openssl(HOST, builder).unwrap().run().await.unwrap();
}
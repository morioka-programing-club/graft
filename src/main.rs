use actix_web::{HttpServer, App, guard};
use actix_web::web::{resource, scope, get, post, Data};
use actix_web::middleware::Compress;
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use env_logger;
use dotenv;
use std::env::var as env;

mod db;
mod web;
mod util;
mod activitypub;
mod auth;
mod error;

use activitypub::is_activitypub_request;

use const_format::concatcp;

const HOST: &str = "localhost";
const PORT: &str = "8088";
const HOST_PORT: &str = concatcp!(HOST, ":", PORT);

lazy_static::lazy_static! {
	static ref DB_NAME: String = env("DB_NAME").unwrap();
}

#[actix_web::main]
async fn main() {
	dotenv::from_filename("GRAFTCONFIG").ok();
	env_logger::init();
	let ssl = env("SSL").map_or(true, |ssl| match ssl.as_str() {
		"false" | "0" => false,
		_ => true
	});

	let db = mongodb::Client::with_uri_str(&env("CLUSTER_URI").unwrap()).await.unwrap();

	let mut server = HttpServer::new(move || {
		// About actix-web:
		// scopes concat before resource URL.
		// use {<name>[:regex]} syntax to match into various strings.
		//
		// About routing:
		// Keep the URLs as short as possible. Don't change any of them unless necessary.
		// When you do need change, remember existing links must be redirected appropriately.
		// Note that these URLs are also referred in the graft context.
		//
		// I recognize "of", "for" and "by" is causing complications below, but other choices also sucks.
		// "inbox" and "outbox" isn't so much intuitive as it's confusing.
		// "posts-for" and "posts-by" don't cover non-post activities.
		// "activities-for" and "activities-by" is verbose and not actually descriptive to anyone new to activities.
		// Bottom line is that there's no word for ActivityPub activities that people are familiar with.
		//
		// About hosting:
		// URLs should make sense when read. "https://example.com/graft/of/user" is a safe bet.
		// Both "https://graft.net/of/user" and "https://gra.ft/of/user" is awkward, but acceptable.
		// "https://example.com/of/user" and "https://graft.example.com/of/user" depends on what `example` is.
		// If your derivative communication system's name, it's equivalent of "graft.net".
		// If a pseudonym signifying your community, it's a nice way to brevity.
		// If it means anything else, I don't recommend it.
		App::new()
			.wrap(Compress::default())
			.app_data(Data::new(db.clone()))
			/*.service(resource("/auth")
				.route(get().to(auth::get_auth))
				.route(post().to(auth::post_auth)),
			)*/
			.route("/new-account", post().to(activitypub::create_account))
			.service(resource("/of/{url_decoration:([^-/]+-)?}{id:[^-/]+}")
				.name("account")
				.route(get().guard(is_activitypub_request).to(activitypub::account))
				.route(get().guard(guard::Not(is_activitypub_request)).to(web::account))
			).service(resource("/by/{url_decoration:([^-/]+-)?}{id:[^-/]+}")
				.name("outbox")
				.route(get().guard(is_activitypub_request).to(activitypub::outbox))
				.route(post().guard(is_activitypub_request).to(activitypub::submit))
			).service(scope("/post/")
				.service(resource(r"{url_decoration:([^-/]+-)?}{time:\d{4}-\d\d-\d\dT\d\d:\d\d(:\d\d(\.\d+)?)?Z}-{id:[^-/]+}")
					.route(get().guard(guard::Not(is_activitypub_request)).to(web::record))
					.route(get().guard(is_activitypub_request).to(activitypub::record))
				).service(resource("{url_decoration:([^-/]+-)?}{id:[^-/]+}")
					.name("post")
					.route(get().guard(guard::Not(is_activitypub_request)).to(web::post))
					.route(get().guard(is_activitypub_request).to(activitypub::post))
				)
			).service(resource("/activity/{id:[^-/]+}")
				.name("activity")
				.route(get().guard(is_activitypub_request).to(activitypub::activity))
			)
			// There shouldn't be any situation where the user wants link to list of revisions.
			// As such, no HTML serving handler or URL decoration is implemented for changelogs.
			// Web client must use Javascript(and/or WebAssembly) to fetch them via ActivityPub interface and show.
			.route("/log/{id}", get().guard(is_activitypub_request).to(activitypub::get_changelog))
			.service(resource("/for/{url_decoration:([^-/]+-)?}{id:[^-/]+}")
				.name("inbox")
				.route(get().guard(guard::Not(is_activitypub_request)).to(web::mentions))
				.route(get().guard(is_activitypub_request).to(activitypub::inbox))
				.route(post().guard(is_activitypub_request).to(activitypub::delivery))
			)
	});

	if ssl {
		// load ssl keys
		let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
		loop {
			match builder.set_private_key_file(env("KEY").unwrap(), SslFiletype::PEM) {
				Ok(_) => break,
				Err(e) => if e.errors()[0].reason() == Some("bad decrypt") {
					eprintln!("Wrong password")
				} else {
					eprintln!("An error happened while verifying certification: {}", e);
					std::process::exit(1)
				}
			}
		}
		builder.set_certificate_chain_file(env("CERT").unwrap()).unwrap();

		server = server.bind_openssl(HOST_PORT, builder).unwrap();
	} else {
		server = server.bind(HOST_PORT).unwrap();
	}

	server.run().await.unwrap();
}
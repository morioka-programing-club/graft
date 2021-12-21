#![feature(fn_traits)]
#![feature(result_flattening)]
#![feature(unboxed_closures)]

use actix_web::middleware::{Compress, Logger};
use actix_web::rt::spawn;
use actix_web::web::{get, post, resource, scope, Data};
use actix_web::{guard, App, HttpServer};
use futures::channel::{mpsc, oneshot};
use futures::lock::Mutex;
use futures::StreamExt;
use once_cell::sync::{Lazy, OnceCell};
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use serde_json::{Map, Value};
use std::env::var as env;
use url::Url;

use deno_core::{op_sync, FsModuleLoader, ModuleId};
use deno_runtime::worker::{MainWorker, WorkerOptions};

mod activitypub;
mod auth;
mod db;
mod error;
mod util;
mod web;

use activitypub::is_activitypub_request;

use const_format::concatcp;

const HOST: &str = "localhost";
const PORT: &str = "8088";
const HOST_PORT: &str = concatcp!(HOST, ":", PORT);

static DB_NAME: Lazy<String> = Lazy::new(|| env("DB_NAME").unwrap());

static SEND_TO_JS_THREAD: OnceCell<Mutex<mpsc::Sender<(String, Map<String, Value>, oneshot::Sender<Result<web::GeneratedHtml, deno_core::anyhow::Error>>)>>> = OnceCell::new();

#[actix_web::main]
async fn main() {
	dotenv::from_filename("GRAFTCONFIG").ok();
	env_logger::init();
	let ssl = env("SSL").map_or(true, |ssl| match ssl.as_str() {
		"false" | "0" => false,
		_ => true
	});

	let db = mongodb::Client::with_uri_str(&env("CLUSTER_URI").unwrap()).await.unwrap();

	let mut deno = MainWorker::bootstrap_from_options(
		Url::parse("graft:svelte_entry_point").unwrap(),
		deno_runtime::permissions::Permissions::allow_all(),
		WorkerOptions {
			bootstrap: deno_runtime::BootstrapOptions {
				apply_source_maps: false,
				args: vec![],
				cpu_count: 1,
				debug_flag: false,
				enable_testing_features: false,
				location: None,
				no_color: false,
				runtime_version: "x".to_string(),
				ts_version: "x".to_string(),
				unstable: false
			},
			extensions: vec![],
			unsafely_ignore_certificate_errors: None,
			root_cert_store: None,
			user_agent: "Deno embbeded in graft".to_string(),
			seed: None,
			js_error_create_fn: None,
			create_web_worker_cb: std::sync::Arc::new(|_| unimplemented!()),
			maybe_inspector_server: None,
			should_break_on_first_statement: false,
			module_loader: std::rc::Rc::new(FsModuleLoader),
			get_error_class_fn: None,
			origin_storage_dir: None,
			blob_store: Default::default(),
			broadcast_channel: Default::default(),
			shared_array_buffer_store: None,
			compiled_wasm_module_store: None
		}
	);

	deno.js_runtime.register_op("get_component", op_sync(|state, _: (), _: ()| Ok(state.take::<String>())));
	deno.js_runtime
		.register_op("get_props", op_sync(|state, _: (), _: ()| Ok(state.take::<Map<String, Value>>())));
	deno.js_runtime.sync_ops_cache();

	let (tx, mut rx) = mpsc::channel(100);
	SEND_TO_JS_THREAD.set(Mutex::new(tx)).unwrap();
	spawn(async move {
		loop {
			if let (Some((name, prop, tx)), new_stream) = rx.into_future().await {
				rx = new_stream;
				let _ = tx.send(web::render_impl(&mut deno, name, prop).await);
			} else {
				break;
			}
		}
	});

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
			.wrap(Logger::default())
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
			).service(scope("/post")
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
				//.route(post().guard(is_activitypub_request).to(activitypub::delivery))
			)
	});

	if ssl {
		// load ssl keys
		let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
		loop {
			match builder.set_private_key_file(env("KEY").unwrap(), SslFiletype::PEM) {
				Ok(_) => break,
				Err(e) => {
					if e.errors()[0].reason() == Some("bad decrypt") {
						eprintln!("Wrong password")
					} else {
						eprintln!("An error happened while verifying certification: {}", e);
						std::process::exit(1)
					}
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

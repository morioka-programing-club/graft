use futures::future;
use actix_web::{HttpServer, App, web, HttpRequest, Responder, error::{self, Error as ActixError}, middleware::Compress};
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use serde_json::Value;
use activitypub::{actor, collection};
use actix_web::http::uri::{Uri, Parts, PathAndQuery};
use std::io::{stdin, stdout, Write};
use actix::prelude::*;
use chrono::{Utc, DateTime};
use env_logger;

mod db;
use db::{DbWrapper, process_senders, process_recievers, expect_single};

mod activitypub_util;
use activitypub_util::{is_activitypub_request, unwrap_short_vec, message_to_json};

#[macro_use]
extern crate lazy_static;

const HOST: &str = "localhost:8088";
const PROTOCOL_HOST: &str = "https://localhost:8088";
const CONTEXT: &str = "http://www.w3.org/ns/activitystreams";

lazy_static! {
	static ref ID: String = "id".to_string();
}

fn is_username(name: &str) -> bool {
	name.starts_with("@")
}

fn group() -> impl Responder {
	// Maybe a web interface
	unimplemented!();
}

fn group_json(req: HttpRequest) -> impl Responder {
	let uri = req.uri();
	let uri_str = &uri.to_string();
	let mut uri_parts = Parts::from(uri.to_owned());
	let mut actor = actor::Group::default();
	uri_parts.path_and_query = Some(PathAndQuery::from_shared((
		String::from("/to/") + &req.match_info().query("actorname")
	).into()).unwrap());

	actor.ap_actor_props.inbox = Uri::from_parts(uri_parts).unwrap().to_string().into();
	actor.ap_actor_props.outbox = (uri_str.clone() + "/all").into();
	actor.object_props.id = Some(Value::from(uri_str.to_owned()));
	actor.object_props.context = Some(Value::from(CONTEXT));
    serde_json::to_string(&actor)
}

fn inbox(req: HttpRequest, db: DbWrapper) -> impl Future<Item = String, Error = ActixError> {
	let uri = req.uri().to_string();
	let mut inbox = collection::OrderedCollection::default();
	inbox.object_props.id = Some(Value::from(uri.to_owned()));
	inbox.object_props.context = Some(Value::from(CONTEXT));

	db.lock().from_err().and_then(move |mut db_locked| {
		let (client, statements) = db_locked.get();
		client.query(&statements.get_inbox, &[&(PROTOCOL_HOST.to_owned() + "/of/" + req.match_info().query("actorname"))])
			.map(message_to_json)
			.and_then(move |mut items| {
				let (client, statements) = db_locked.get();
				let id = items.get_mut(&*ID).unwrap().as_i64().unwrap();

				db::get_senders_and_recievers(client, statements, id).and_then(move |(senders, recievers)| {
					items.insert("actor".to_string(), unwrap_short_vec(senders).into());
					items.insert("to".to_string(), unwrap_short_vec(recievers).into());
					Ok(items)
				})
			}).collect().and_then(move |items| {
				inbox.collection_props.items = items.into();
				serde_json::to_string(&inbox).map_err(|_| panic!("JSON serialization error"))
			}).map_err(error::ErrorInternalServerError)
	})
}

fn outbox(req: HttpRequest, db: DbWrapper) -> impl Future<Item = String, Error = ActixError> {
	let uri = req.uri().to_string();
	let mut outbox = collection::OrderedCollection::default();
	outbox.object_props.id = Some(Value::from(uri.to_owned()));
	outbox.object_props.context = Some(Value::from(CONTEXT));

	db.lock().from_err().and_then(move |mut db_locked| {
		let (client, statements) = db_locked.get();
		client.query(&statements.get_outbox, &[&(PROTOCOL_HOST.to_owned() + "/of/" + req.match_info().query("actorname"))])
			.map(message_to_json)
			.and_then(move |mut items| {
				let (client, statements) = db_locked.get();
				let id = items.get_mut(&*ID).unwrap().as_i64().unwrap();

				db::get_senders_and_recievers(client, statements, id).and_then(move |(senders, recievers)| {
					items.insert("actor".to_string(), unwrap_short_vec(senders).into());
					items.insert("to".to_string(), unwrap_short_vec(recievers).into());
					Ok(items)
				})
			}).collect().and_then(move |items| {
				outbox.collection_props.items = items.into();
				serde_json::to_string(&outbox).map_err(|_| panic!("JSON serialization error"))
			}).map_err(error::ErrorInternalServerError)
	})
}

fn single_post(req: HttpRequest, db: DbWrapper) -> Box<Future<Item = String, Error = ActixError>> {
	let id = match i64::from_str_radix(req.match_info().query("id"), 10) {
		Ok(id) => id,
		Err(e) => return Box::new(future::err(error::ErrorBadRequest(e)))
	};

	Box::new(db.lock().from_err().and_then(move |mut db_locked| {
		let (client, statements) = db_locked.get();
		client.query(&statements.get_message, &[&id])
			.map(message_to_json)
			.collect()
			.map_err(error::ErrorInternalServerError)
			.and_then(expect_single("No post found for the ID requested", "Internal error")).and_then(move |mut items| {
				let (client, statements) = db_locked.get();

				db::get_senders_and_recievers(client, statements, id)
					.map_err(error::ErrorInternalServerError).and_then(move |(senders, recievers)| {
					items.insert("actor".to_string(), unwrap_short_vec(senders).into());
					items.insert("to".to_string(), unwrap_short_vec(recievers).into());
					Ok(items)
				})
			}).and_then(move |message| {
				serde_json::to_string(&message).map_err(|_| panic!("JSON serialization error"))
			})
	}))
}

fn changelog(req: HttpRequest, db: DbWrapper) -> Box<Future<Item = String, Error = ActixError>> {
	let id = match i64::from_str_radix(req.match_info().query("id"), 10) {
		Ok(id) => id,
		Err(e) => return Box::new(future::err(error::ErrorBadRequest(e)))
	};

	Box::new(db.lock().from_err().and_then(move |mut db_locked| {
		let (client, statements) = db_locked.get();
		client.query(&statements.get_changelog, &[&id])
			.map(|row| row.get::<usize, i64>(0).into())
			.collect()
			.map_err(error::ErrorInternalServerError)
			.and_then(move |history| {
				serde_json::to_string(&Value::Array(history)).map_err(|_| panic!("JSON serialization error"))
			})
	}))
}

fn create(req: HttpRequest, db: DbWrapper) -> Box<Future<Item = String, Error = ActixError>> {
	Box::new(db.lock().from_err().and_then(move |mut db_locked| {
		let mut name = req.match_info().query("actorname").to_owned();
		let isuser = is_username(&name);
		if isuser {name.remove(0);}
		let (client, statements) = db_locked.get();
		client.execute(&statements.create_actor, &[
			&if isuser {db::ActorVariant::User} else {db::ActorVariant::Group},
			&req.uri().to_string()
		]).join(future::ok(isuser))
			.map(|(_, isuser)| if isuser {"User"} else {"Group"}.to_owned() + " succesfully created")
			.map_err(error::ErrorInternalServerError)
	}))
}

fn delete() -> impl Responder {
	"Deleting a group is not supported"
}

fn post(json: web::Json<Value>, db: DbWrapper) -> Box<Future<Item = String, Error = ActixError>> {
	if !json["type"].is_string() {
		return Box::new(future::err(error::ErrorBadRequest("Non-activitystream object recieved")))
	}

	match json["type"].as_str().unwrap() {
		"Create" => {
			Box::new(db.lock().from_err().join(future::ok(json)).and_then(|(mut db_locked, json)| {
				let (client, statements) = db_locked.get();
				client.query(&statements.create_message, &[&json["object"]["content"].as_str(), &Utc::now(), &None::<i64>])
					.map_err(error::ErrorInternalServerError)
					.collect().join(future::ok(json))
					.and_then(move |(id, mut json)| {
						process_senders(json["actor"].take(), id[0].get(0), db.clone())
							.join(process_recievers(json["to"].take(), id[0].get(0), db.clone()))
					})
					.map(|_| "".to_string())
			}))
		},
		"Update" => {
			Box::new(db.lock().from_err().join(future::ok(json)).and_then(|(mut db_locked, json)| {
				let (client, statements) = db_locked.get();
				client.query(&statements.get_message, &[&json["object"]["id"].as_i64()])
					.collect()
					.map_err(error::ErrorInternalServerError)
					.and_then(expect_single("No post found for the ID requested", "Internal error"))
					.and_then(move |message| {
						let id = message.get::<&str, i64>("id");
						let logid_original = message.get::<&str, Option<i64>>("changelog");
						let fut: Box<Future<Item = i64, Error = ActixError>> = if let None = logid_original {
							let (client, statements) = db_locked.get();
							Box::new(client.query(&statements.version_message, &[&id])
								.collect()
								.map_err(error::ErrorInternalServerError)
								.and_then(expect_single("Internal error", "Internal error"))
								.and_then(|row| Ok(row.get::<usize, i64>(0))))
						} else {
							Box::new(future::ok(logid_original.unwrap()))
						};
						fut.and_then(move |logid| {
							let (client, statements) = db_locked.get();
							client.query(&statements.create_message, &[
								&message.get::<&str, &str>("content"), &message.get::<&str, DateTime<Utc>>("time"), &logid
							]).collect().map_err(error::ErrorInternalServerError)
							.and_then(expect_single("Internal error", "Internal error"))
							.and_then(move |old| {
								let (client, statements) = db_locked.get();
								client.query(&statements.add_message_version, &[&logid, &old.get::<&str, i64>("id")])
									.collect().map_err(error::ErrorInternalServerError)
									.and_then(move |_| {
										let (client, statements) = db_locked.get();
										client.query(&statements.update_message, &[&id, &json["object"]["content"].as_str(), &Utc::now(), &logid])
											.collect().map_err(error::ErrorInternalServerError)
									})
							})
						})
					}).map(|_| "".to_string())
			}))
		},
		_ => Box::new(future::err(error::ErrorBadRequest("Unknown object type")))
	}
}

fn user() -> impl Responder {
	// Maybe a web interface
	unimplemented!();
}

fn delete_user() -> impl Responder {
	"Deleting a user is not supported"
}

fn user_json(req: HttpRequest) -> impl Responder {
	let uri = req.uri();
	let uri_str = &uri.to_string();
	let mut uri_parts = Parts::from(uri.to_owned());
	let mut actor = actor::Group::default();
	uri_parts.path_and_query = Some(PathAndQuery::from_shared((
		String::from("/to/") + &req.match_info().query("actorname")
	).into()).unwrap());

	actor.ap_actor_props.inbox = Uri::from_parts(uri_parts).unwrap().to_string().into();
	actor.ap_actor_props.outbox = (uri_str.clone() + "/all").into();
	actor.object_props.id = Some(Value::from(uri_str.to_owned()));
	actor.object_props.context = Some(Value::from(CONTEXT));
    serde_json::to_string(&actor)
}

fn main() {
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

	print!("Input PostgreSQL user name: ");
	stdout().flush().unwrap();
	let mut user_name = String::new();
    stdin().read_line(&mut user_name).expect("Failed to read line");
	let len = user_name.len();

	let future = db::init(&user_name[0..len-1], "graft").and_then(|db| {
		HttpServer::new(move || {
			/*
			 * About actix-web
			 * scopes concat before resource URL
			 * use {<name>[:regex]} syntax to match into various strings
			 */

			App::new()
				.wrap(Compress::default())
				.register_data(db.clone())
				.service(
					web::scope("/of/{actorname:[^/@][^/]*}")
						.service(web::resource("")
							.route(web::get().to(group))
							.route(web::post().to(group_json))
							.route(web::put().to_async(create))
							.route(web::delete().to(delete))
						).service(web::resource("/all")
							.route(web::get().to_async(outbox))
							.route(web::post().guard(is_activitypub_request).to_async(post))
						)
				).service(
					web::scope("/of/{actorname:@[^/]+}")
						.service(web::resource("")
							.route(web::get().to(user))
							.route(web::post().to(user_json))
							.route(web::put().to_async(create))
							.route(web::delete().to(delete_user))
						).service(web::resource("/all")
							.route(web::get().to_async(outbox))
							.route(web::post().guard(is_activitypub_request).to_async(post))
						)
				).service(web::resource("/post/{id}")
					.route(web::get().to_async(single_post))
				).service(web::resource("/log/{id}")
					.route(web::get().to_async(changelog))
				).service(web::resource("/to/{actorname}")
					.route(web::get().to_async(inbox))
					.route(web::post().to_async(inbox))
				)
		}).bind_ssl(HOST, builder)?.start();
		Ok(())
	});

	Arbiter::spawn(future.map_err(|e| {
		eprint!("Following error occurred: {:?}", e);
		let inner_error = e.into_inner();
		match inner_error {
			Some(e) => eprintln!("{}", e),
			None => eprintln!()
		}
	}));
	let sys = System::builder().stop_on_panic(false).build();
	let result = sys.run();
	match result {
		Ok(_) => {},
		Err(e) => panic!(e)
	}
}
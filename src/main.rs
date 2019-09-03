use futures::future;
use actix_web::{HttpServer, App, web, HttpRequest, Responder, error::{self, Error as ActixError}, dev::RequestHead};
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use serde_json::{Value, Map};
use activitypub::{actor, collection};
use actix_web::http::uri::{Uri, Parts, PathAndQuery};
use std::io::{stdin, stdout, Write};
use actix::prelude::*;
use chrono::Utc;

mod db;
use db::{DbWrapper, process_senders, process_recievers};

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
		String::from("/to/") + &req.match_info().query("groupname")
	).into()).unwrap());

	actor.ap_actor_props.inbox = Uri::from_parts(uri_parts).unwrap().to_string().into();
	actor.ap_actor_props.outbox = (uri_str.clone() + "/all").into();
	actor.object_props.id = Some(Value::from(uri_str.to_owned()));
	actor.object_props.context = Some(Value::from("https://www.w3.org/ns/activitystreams"));
    serde_json::to_string(&actor)
}

fn inbox(req: HttpRequest, db: DbWrapper) -> impl Future<Item = String, Error = ActixError> {
	let uri = req.uri().to_string();
	let mut inbox = collection::OrderedCollection::default();
	inbox.object_props.id = Some(Value::from(uri.to_owned()));
	inbox.object_props.context = Some(Value::from("https://www.w3.org/ns/activitystreams"));

	db.lock().from_err().and_then(move |mut db_locked| {
		let (client, statements) = db_locked.get();
		client.query(&statements.get_inbox, &[&req.match_info().query("groupname")])
			.map(|row| row.columns().into_iter()
				.map(|col| {
					let name = col.name();
					(String::from(name), db::into_value(&row, name, col.type_()))
				})
				.collect::<Map<String, Value>>())
			.collect().and_then(move |items| {
				inbox.collection_props.items = items.into();
				serde_json::to_string(&inbox).map_err(|_| panic!("JSON serialization error"))
			}).map_err(error::ErrorInternalServerError)
	})
}

fn outbox(req: HttpRequest, db: DbWrapper) -> impl Future<Item = String, Error = ActixError> {
	let uri = req.uri().to_string();
	let mut outbox = collection::OrderedCollection::default();
	outbox.object_props.id = Some(Value::from(uri.to_owned()));
	outbox.object_props.context = Some(Value::from("https://www.w3.org/ns/activitystreams"));

	db.lock().from_err().and_then(move |mut db_locked| {
		let (client, statements) = db_locked.get();
		client.query(&statements.get_outbox, &[&req.match_info().query("groupname")])
			.map(|row| row.columns().into_iter()
				.map(|col| {
					let name = col.name();
					(String::from(name), db::into_value(&row, name, col.type_()))
				})
				.collect::<Map<String, Value>>())
			.collect().and_then(move |items| {
				outbox.collection_props.items = items.into();
				serde_json::to_string(&outbox).map_err(|_| panic!("JSON serialization error"))
			}).map_err(error::ErrorInternalServerError)
	})
}

fn create(req: HttpRequest, db: DbWrapper) -> impl Future<Item = &'static str, Error = ActixError> {
	db.lock().from_err().and_then(move |mut db_locked| {
		let (client, statements) = db_locked.get();
		client.execute(&statements.create_group, &[
			&db::ActorVariant::Group,
			&req.match_info().query("groupname")
		]).map(|_| "Group succesfully created").map_err(error::ErrorInternalServerError)
	})
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
				client.query(&statements.create_message, &[&json["object"]["content"].as_str(), &Utc::now()])
					.map_err(error::ErrorInternalServerError)
					.collect().join(future::ok(json))
					.and_then(move |(id, mut json)| {
						process_senders(json["actor"].take(), id[0].get(0), db.clone())
							.join(process_recievers(json["to"].take(), id[0].get(0), db.clone()))
					})
					.map(|_| "".to_string())
			}))
		},
		_ => Box::new(future::err(error::ErrorBadRequest("Unknown object type")))
	}
}

fn main() {
    // load ssl keys
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder.set_private_key_file("key.pem", SslFiletype::PEM).unwrap();
    builder.set_certificate_chain_file("cert.pem").unwrap();

	print!("Input PostgreSQL user name: ");
	stdout().flush().unwrap();
	let mut user_name = String::new();
    stdin().read_line(&mut user_name).expect("Failed to read line");
	let len = user_name.len();

	let future = db::init(&user_name[0..len-1]).and_then(|db| {
		HttpServer::new(move || {
			App::new()
				.register_data(db.clone())
				.service(
					web::scope("/of/{groupname}")
						.service(web::resource("")
							.route(web::get().to(group))
							.route(web::post().to(group_json))
							.route(web::put().to_async(create))
							.route(web::delete().to(delete))
						).service(web::resource("/all")
							.route(web::get().to_async(outbox))
							.route(web::post().guard(|head: &RequestHead| {
								match head.headers.get("Content-Type") {
									Some(v) => v == "application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\""
										|| v == "application/activity+json",
									None => false
								}
							}).to_async(post))
						)
				).service(web::resource("/to/{groupname}")
					.route(web::get().to_async(inbox))
					.route(web::post().to_async(inbox))
				)
		}).bind_ssl("127.0.0.1:8088", builder)?.start();
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
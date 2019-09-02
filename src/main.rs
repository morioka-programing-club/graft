use std::error::Error;
use futures::future;
use actix_web::{HttpServer, App, web, HttpRequest, Responder, error::{self, Error as ActixError}, dev::RequestHead};
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use serde_json::{Value, Map};
use activitypub::{actor, collection};
use actix_web::http::{StatusCode, uri::{Uri, Parts, PathAndQuery}};
use tokio_postgres::{connect, NoTls, Statement, Client, Row, types::{Type, Kind, IsNull, ToSql}};
use futures_locks::Mutex;
use std::io::{self, ErrorKind, stdin, stdout, Write};
use actix::prelude::*;
use chrono::Utc;

#[derive(Debug)]
enum ActorVariant {
	User,
	Group
}

// Manually expanded https://github.com/sfackler/rust-postgres-derive since it didn't work with tokio-postgres
impl ToSql for ActorVariant {
	fn to_sql(&self, _type: &Type, buf: &mut Vec<u8>) -> Result<IsNull, Box<Error + Sync + Send>> {
		let s = match self {
            ActorVariant::User => "member",
			ActorVariant::Group => "organization"
        };

        buf.extend_from_slice(s.as_bytes());
        Ok(IsNull::No)
    }

    fn accepts(type_: &Type) -> bool {
		if type_.name() != "actors_available" {
            return false;
        }

        match *type_.kind() {
            Kind::Enum(ref variants) => {
                if variants.len() != 2 {
                    return false;
                }

                variants.iter().all(|v| {
                    match &**v {
                        "member" => true,
                        "organization" => true,
                        _ => false
                    }
                })
            }
            _ => false
        }
    }

    fn to_sql_checked(&self, type_: &Type, buf: &mut Vec<u8>) -> Result<IsNull, Box<Error + Sync + Send>> {
		self.to_sql(type_, buf)
	}
}

fn into_value(row: &Row, name: &str, col_type: &Type) -> Value {
	macro_rules! from_sql {
		($(($sql_type:ident, $type_to:ty)),*) => {
			match col_type {
				$(&Type::$sql_type => row.get::<&str, $type_to>(name).into(),)*
				_ => panic!("Specified SQL cell's type is not compatible to JSON")
			}
		}
	}
	from_sql![
		(CHAR, i8),
		(INT2, i16),
		(INT4, i32),
		(INT8, i64),
		(OID, u32),
		(FLOAT4, f32),
		(FLOAT8, f64),
		(BYTEA, &[u8]),
		(TEXT, &str),
		(BOOL, bool)
	]
}

fn group() -> impl Responder {
	// Maybe a web interface
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
					(String::from(name), into_value(&row, name, col.type_()))
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
					(String::from(name), into_value(&row, name, col.type_()))
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
			&ActorVariant::Group,
			&req.match_info().query("groupname")
		]).map(|_| "Group succesfully created").map_err(error::ErrorInternalServerError)
	})
}

fn delete() -> impl Responder {
	"Deleting a group is not supported"
}

fn process_senders(json: Value, id: u32, db: DbWrapper) -> Box<Future<Item = (), Error = ActixError>> {
	match json {
		Value::String(str) => {
			Box::new(db.lock().from_err().join(future::ok(str)).and_then(move |(mut db_locked, str)| {
				let (client, statements) = db_locked.get();
				client.execute(&statements.send_message, &[&str, &id])
					.map(|_| ()).map_err(error::ErrorInternalServerError)
			}))
		},
		Value::Object(obj) => {
			Box::new(db.lock().from_err().join(future::ok(obj)).and_then(move |(mut db_locked, obj)| {
				let (client, statements) = db_locked.get();
				client.execute(&statements.send_message, &[&obj["id"].as_str(), &Some(id)])
					.map(|_| ()).map_err(error::ErrorInternalServerError)
			}))
		},
		Value::Array(arr) => Box::new(
			future::join_all(arr.to_owned().into_iter().map(move |el| process_senders(el, id, db.clone()))).map(|_| ())
				.map_err(|e| error::InternalError::new(e, StatusCode::INTERNAL_SERVER_ERROR).into())
		),
		_ => Box::new(future::err(error::ErrorBadRequest("Invaild actor")))
	}
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
					.and_then(move |(id, mut json)| process_senders(json["actor"].take(), id[0].get(0), db.clone()))
					.map(|_| "".to_string())
			}))
		},
		_ => Box::new(future::err(error::ErrorBadRequest("Unknown object type")))
	}
}

struct Db {
	client: Client,
	statements: Statements
}

impl Actor for Db {
    type Context = Context<Self>;
}

impl Db {
	fn get(&mut self) -> (&mut Client, &Statements) {
		(&mut self.client, &self.statements)
	}
}

struct Statements {
	get_inbox: Statement,
	get_outbox: Statement,
	create_message: Statement,
	send_message: Statement,
	create_group: Statement,
	delete_group: Statement
}

type DbWrapper = web::Data<Mutex<Db>>;

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

	let future = connect(&(String::from("postgres://") + &user_name[0..len-1] + "@localhost/graft"), NoTls)
			.map_err(|e| io::Error::new(ErrorKind::Other, e))
			.and_then(move |(mut cl, conn)| {
		Arbiter::spawn(conn.map_err(|e| panic!("{}", e)));
		future::join_all(vec![
			// Insert SQL statements here
			cl.prepare("SELECT * FROM messages WHERE reciever = $1 ORDER BY ctime;"), // get inbox
			cl.prepare("SELECT * FROM messages WHERE sender = $1 ORDER BY ctime;"), // get outbox
			cl.prepare("INSERT INTO messages (content, ctime, mtime) VALUES ($1, $2, $2) RETURNING id;"), // create message
			cl.prepare(r#"
INSERT INTO messages_sent (actor, message) VALUES ($2, $1);
INSERT INTO messages_recieved (actor, message) VALUES ($3, $1);"#), // send message
			cl.prepare("INSERT INTO actors (actortype, id) VALUES ($1, $2);"), // create actor
			cl.prepare("DELETE FROM actors WHERE actortype = 'organization' AND id = $1;") // delete group
		]).and_then(move |statements| {
			let mut iter = statements.into_iter();
			Ok(Db {
				client: cl,
				statements: Statements {
					get_inbox: iter.next().unwrap(),
					get_outbox: iter.next().unwrap(),
					create_message: iter.next().unwrap(),
					send_message: iter.next().unwrap(),
					create_group: iter.next().unwrap(),
					delete_group: iter.next().unwrap()
				}
			})
		}).map_err(|e| io::Error::new(ErrorKind::Other, e))
	}).and_then(|db| {
		println!("SQL Statements prepared successfully");

		let db = web::Data::new(Mutex::new(db));
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
use futures::{select, FutureExt, SinkExt};

use actix_web::error::Error as ActixError;
use actix_web::Responder;
use futures::channel::oneshot::channel;
use log::debug;
use serde_json::{Map, Value};

use deno_core::anyhow::Context;
use deno_runtime::worker::MainWorker;

use crate::error::internal_error;
use crate::util::resolve_component;
use crate::SEND_TO_JS_THREAD;

mod handler;

pub use handler::*;

#[derive(serde::Deserialize)]
pub struct GeneratedHtml {
	head: String,
	html: String
}

async fn render(name: &'static str, prop: Map<String, Value>) -> Result<impl Responder, ActixError> {
	let (tx, rx) = channel();
	SEND_TO_JS_THREAD
		.get()
		.unwrap()
		.lock()
		.await
		.send((resolve_component(name), prop, tx))
		.await
		.map_err(internal_error)?;
	rx.await.map_err(internal_error)?.map_err(internal_error).map(|GeneratedHtml { head, html }| {
		format!("<html><head>{}</head><body>{}</body></html>", head, html)
			.customize()
			.insert_header(("Content-Type", "text/html; charset=utf-8"))
	})
}

pub async fn render_impl(deno: &mut MainWorker, name: String, prop: Map<String, Value>) -> Result<GeneratedHtml, deno_core::anyhow::Error> {
	let state = deno.js_runtime.op_state();
	{
		let mut state = state.borrow_mut();
		state.put(name);
		state.put(prop);
	}

	// TODO: turn this into a module when JsRuntime::mod_evaluate is fixed to return the value
	let promise = deno.js_runtime.execute_script(
		"graft:svelte_entry_point",
		r#"import(Deno.core.opSync("get_component")).then(module => module.default.render(Deno.core.opSync("get_props")))"#
	)?;
	let result = deno.js_runtime.resolve_value(promise).await?;

	let mut scope = deno.js_runtime.handle_scope();
	let result = v8::Local::new(&mut scope, result);
	serde_v8::from_v8(&mut scope, result).map_err(deno_core::anyhow::Error::new).with_context(|| {
		r#"failed to read generated HTML in Rust code. Expected an object with "head" and "html" keys, got "#.to_string()
			+ &serde_v8::from_v8::<Value>(&mut scope, result).unwrap().to_string()
	})
}

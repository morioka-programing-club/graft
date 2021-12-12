use futures::{select, FutureExt, SinkExt};

use actix_web::Responder;
use actix_web::error::Error as ActixError;
use futures::channel::oneshot::channel;
use serde_json::{Map, Value};
use log::debug;

use deno_runtime::worker::MainWorker;

use crate::{ENTRY_MODULE, SEND_TO_JS_THREAD};
use crate::error::internal_error;

mod handler;

pub use handler::*;

#[derive(serde::Deserialize)]
pub struct GeneratedHtml {
	head: String,
	html: String
}

async fn render(name: &'static str, prop: Map<String, Value>) -> Result<impl Responder, ActixError> {
	let (tx, rx) = channel();
	SEND_TO_JS_THREAD.get().unwrap().lock().await.send((name, prop, tx)).await.map_err(internal_error)?;
	rx.await.map_err(internal_error)?.map_err(internal_error)
		.map(|GeneratedHtml {head, html}| format!("<html><head>{}</head><body>{}</body></html>", head, html)
			.with_header(("Content-Type", "text/html; charset=utf-8")))
}

pub async fn render_impl(deno: &mut MainWorker, name: &'static str, prop: Map<String, Value>) -> Result<GeneratedHtml, deno_core::anyhow::Error> {
	let state = deno.js_runtime.op_state();
	let mut state = state.borrow_mut();
	state.put(name);
	state.put(prop);
	
	// MainWorker::execute_main_module was not patched to return a value, so we copy the modified version here instead
	let mut receiver = deno.js_runtime.mod_evaluate(*ENTRY_MODULE.get().unwrap());
	let result = select! {
		maybe_result = &mut receiver => {
			debug!("received module evaluate {:#?}", maybe_result);
			maybe_result
		}

		event_loop_result = deno.run_event_loop(false).fuse() => {
			event_loop_result?;
			receiver.await
		}
	}.expect("Module evaluation result not provided.")?;
	
	let mut scope = deno.js_runtime.handle_scope();
	let result = v8::Local::new(&mut scope, result);
	serde_v8::from_v8(&mut scope, result).map_err(deno_core::anyhow::Error::new)
}
use std::future::Future;

use futures::TryFutureExt;

use actix_web::Responder;
use actix_web::error::Error as ActixError;
use serde_json::{Map, Value};

use nodejs::neon::context::Context;
use nodejs::neon::handle::Handle;
use nodejs::neon::object::Object;
use nodejs::neon::result::Throw;
use nodejs::neon::types::{JsFunction, JsObject, JsString};

use crate::error::internal_error;

mod handler;
pub mod node;

pub use handler::*;

async fn render_html(name: &'static str, prop: Map<String, Value>) -> Result<impl Responder, ActixError> {
	render(name, prop).map_ok(|(head, body)| format!("<html><head>{}</head><body>{}</body></html>", head, body))
		.await.map(|html| html.with_header(("Content-Type", "text/html; charset=utf-8")))
}

fn render(name: &'static str, prop: Map<String, Value>) -> impl Future<Output = Result<(String, String), ActixError>> {
	node::send_task_with_return_value(move |ctx| {
		let template: Handle<JsObject> = ctx.global().get(ctx, name)?.downcast_or_throw(ctx)?;
		let renderfn: Handle<JsFunction> = template.get(ctx, "render")?.downcast_or_throw(ctx)?;
		let prop = neon_serde::to_value(ctx, &prop).map_err(|err| throw_neon_serde_error(ctx, err))?;
		destructure_svelte_html(renderfn.call(ctx, template, [prop])?.downcast_or_throw(ctx)?, ctx)
	}).map_err(internal_error)
}

fn destructure_svelte_html<'a>(html: Handle<JsObject>, ctx: &mut impl Context<'a>) -> Result<(String, String), Throw> {
	Ok((
		html.get(ctx, "head")?.downcast_or_throw::<JsString, _>(ctx)?.value(ctx),
		html.get(ctx, "html")?.downcast_or_throw::<JsString, _>(ctx)?.value(ctx)
	))
}

// Not sure why this conversion was removed in neon_serde; probably something about unsafeness
fn throw_neon_serde_error<'a>(ctx: &mut impl Context<'a>, err: neon_serde::errors::Error) -> Throw {
    if let neon_serde::errors::ErrorKind::Js(_) = *err.kind() {
        return Throw; // it's already thrown
    };
    ctx.throw_error::<_, std::convert::Infallible>(format!("{:?}", err)).err().unwrap()
}
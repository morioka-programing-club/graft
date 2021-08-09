use std::future::Future;

use futures::FutureExt;

use nodejs::neon::context::{Context, TaskContext};
use nodejs::neon::handle::{Handle, Managed};
use nodejs::neon::result::Throw;
use nodejs::neon::types::Value;

pub fn send_task_with_return_value<R>(cb: impl FnOnce(&mut TaskContext) -> Result<R, Throw> + Send + 'static) ->
	impl Future<Output = Result<R, String>>
where
	R: Send + 'static
{
	let (tx, rx) = futures::channel::oneshot::channel();
    nodejs::channel().send(move |mut ctx| {
		let ret = ctx.try_catch(cb).map_err(|err| err.to_string(&mut ctx).unwrap().value(&mut ctx));
		// Add waiting for promise here
		let _ = tx.send(ret);
		Ok(())
    });
    rx.map(Result::unwrap)
}

pub fn eval<T, R>(src: impl AsRef<str> + Send + 'static, convert: impl FnOnce(Handle<T>, &mut TaskContext) -> R + Send + 'static) ->
	impl Future<Output = Result<R, String>>
where
	T: Managed + Value,
	R: Send + 'static
{
	send_task_with_return_value(|ctx| {
		let src = ctx.string(src);
		Ok(convert(nodejs::neon::reflect::eval(ctx, src)?.downcast_or_throw(ctx)?, ctx))
	})
}
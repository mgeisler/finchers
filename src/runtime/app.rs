//! The components to construct an asynchronous HTTP service from the `Endpoint`.

use std::boxed::PinBox;
use std::io;
use std::mem::PinMut;
use std::sync::Arc;
use std::time;

use futures::{self, Async, Future, Poll};
use http::header::{self, HeaderValue};
use http::{Request, Response};
use hyper::body::Body;
use hyper::service::{NewService, Service};
use scoped_tls::scoped_thread_local;
use slog::{kv, o, slog_b, slog_info, slog_kv, slog_log, slog_record, slog_record_static, Logger};

use futures_core::future::TryFuture;
use futures_util::compat::{Compat, TokioDefaultExecutor};
use futures_util::try_future::{IntoFuture, TryFutureExt};

use error::{Error, HttpError, NoRoute};
use generic::Either;
use input::body::ReqBody;
use input::{with_set_cx, Input};
use output::payloads::Once;
use output::Responder;
use runtime::AppEndpoint;

/// A factory of HTTP service which wraps an `Endpoint`.
#[derive(Debug)]
pub struct App<E: AppEndpoint> {
    data: Arc<AppData<E>>,
}

#[derive(Debug)]
struct AppData<E: AppEndpoint> {
    endpoint: E,
    logger: Logger,
}

impl<E: AppEndpoint> App<E> {
    /// Create a new `App` from the provided components.
    pub fn new(endpoint: E, logger: Logger) -> App<E> {
        App {
            data: Arc::new(AppData { endpoint, logger }),
        }
    }
}

impl<E: AppEndpoint> NewService for App<E> {
    type ReqBody = Body;
    type ResBody = Either<Once<String>, <E::Output as Responder>::Body>;
    type Error = io::Error;
    type Service = AppService<E>;
    type InitError = io::Error;
    type Future = futures::future::FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        futures::future::ok(AppService {
            data: self.data.clone(),
        })
    }
}

/// An asynchronous HTTP service which holds an `Endpoint`.
///
/// The value of this type is generated by `NewEndpointService`.
#[derive(Debug)]
pub struct AppService<E: AppEndpoint> {
    data: Arc<AppData<E>>,
}

impl<E: AppEndpoint> Service for AppService<E> {
    type ReqBody = Body;
    type ResBody = Either<Once<String>, <E::Output as Responder>::Body>;
    type Error = io::Error;
    type Future = AppServiceFuture<TokioCompat<E::Future>>;

    fn call(&mut self, request: Request<Self::ReqBody>) -> Self::Future {
        let request = request.map(ReqBody::from_hyp);
        let logger = self.data.logger.new(o!{
            "method" => request.method().to_string(),
            "path" => request.uri().path().to_owned(),
        });
        let mut input = Input::new(request);
        let in_flight = {
            let input = unsafe { PinMut::new_unchecked(&mut input) };
            self.data.endpoint.apply(input).map(tokio_compat)
        };

        AppServiceFuture {
            in_flight,
            input,
            logger,
            start: time::Instant::now(),
        }
    }
}

#[allow(missing_docs)]
#[allow(missing_debug_implementations)]
pub struct AppServiceFuture<T> {
    in_flight: Option<T>,
    input: Input,
    logger: Logger,
    start: time::Instant,
}

impl<T> AppServiceFuture<T> {
    fn handle_error(&self, err: &dyn HttpError) -> Response<Once<String>> {
        let mut response = Response::new(Once::new(format!("{:#}", err)));
        *response.status_mut() = err.status_code();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        err.headers(response.headers_mut());
        response
    }
}

impl<T> Future for AppServiceFuture<T>
where
    T: Future<Error = Error>,
    T::Item: Responder,
{
    type Item = Response<Either<Once<String>, <T::Item as Responder>::Body>>;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let polled = {
            let logger = &self.logger;
            let in_flight = &mut self.in_flight;
            let input = &mut self.input;
            LOGGER.set(logger, || match in_flight {
                Some(ref mut f) => {
                    let input = unsafe { PinMut::new_unchecked(input) };
                    with_set_cx(input, || Some(f.poll()))
                }
                None => None,
            })
        };

        let output = match polled {
            Some(Ok(Async::NotReady)) => return Ok(Async::NotReady),
            Some(Ok(Async::Ready(out))) => {
                let input = unsafe { PinMut::new_unchecked(&mut self.input) };
                out.respond(input)
                    .map(|res| res.map(Either::Right))
                    .map_err(Into::into)
            }
            Some(Err(err)) => Err(err),
            None => Err(NoRoute.into()),
        };

        let mut response = output.unwrap_or_else(|err| self.handle_error(&*err).map(Either::Left));

        response
            .headers_mut()
            .entry(header::SERVER)
            .unwrap()
            .or_insert(HeaderValue::from_static(concat!(
                "finchers-runtime/",
                env!("CARGO_PKG_VERSION")
            )));

        slog_info!(self.logger, "{} ({} ms)", response.status(), {
            let end = time::Instant::now();
            let duration = end - self.start;
            duration.as_secs() * 10 + u64::from(duration.subsec_nanos()) / 1_000_000
        });

        Ok(Async::Ready(response))
    }
}

// ==== TokioCompat ====

type TokioCompat<F> = Compat<PinBox<IntoFuture<F>>, TokioDefaultExecutor>;

fn tokio_compat<F: TryFuture>(future: F) -> TokioCompat<F> {
    PinBox::new(future.into_future()).compat(TokioDefaultExecutor)
}

// ==== Logger ====

scoped_thread_local!(static LOGGER: Logger);

/// Execute a closure with the reference to `Logger` associated with the current scope.
pub fn with_logger<F, R>(f: F) -> R
where
    F: FnOnce(&Logger) -> R,
{
    LOGGER.with(|logger| f(logger))
}

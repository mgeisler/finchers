//! The components for using the implementor of `Endpoint` as an HTTP `Service`.

use futures::future;
use http::{Request, Response};
use hyper::body::Body;
use tower_service::NewService;

use self::app_endpoint::{AppEndpoint, Lift};
pub use self::app_payload::AppPayload;
use self::app_service::AppService;

use error::Never;

/// A trait which compose the trait bounds representing that
/// the implementor is able to use as an HTTP service.
pub trait IsAppEndpoint: for<'a> AppEndpoint<'a> {}

impl<E> IsAppEndpoint for E where for<'a> E: AppEndpoint<'a> {}

mod app_endpoint {
    use futures::Future;

    use common::Tuple;
    use endpoint::{ApplyContext, ApplyResult, Endpoint};
    use error::Error;
    use output::Output;

    pub trait AppEndpoint<'a>: Send + Sync + 'static {
        type Output: Tuple + Output;
        type Future: Future<Item = Self::Output, Error = Error> + Send + 'a;
        fn apply_app(&'a self, cx: &mut ApplyContext<'_>) -> ApplyResult<Self::Future>;
    }

    impl<'a, E> AppEndpoint<'a> for E
    where
        E: Endpoint<'a> + Send + Sync + 'static,
        E::Output: Output,
        E::Future: Send,
    {
        type Output = E::Output;
        type Future = E::Future;

        #[inline]
        fn apply_app(&'a self, cx: &mut ApplyContext<'_>) -> ApplyResult<Self::Future> {
            self.apply(cx)
        }
    }

    #[derive(Debug)]
    pub struct Lift<E>(pub(super) E);

    impl<'a, E> Endpoint<'a> for Lift<E>
    where
        E: AppEndpoint<'a>,
    {
        type Output = E::Output;
        type Future = E::Future;

        #[inline]
        fn apply(&'a self, cx: &mut ApplyContext<'_>) -> ApplyResult<Self::Future> {
            self.0.apply_app(cx)
        }
    }
}

/// A wrapper struct for lifting the instance of `Endpoint` to an HTTP service.
///
/// # Safety
///
/// The implementation of `NewService` for this type internally uses unsafe block
/// with an assumption that `self` always outlives the returned future.
/// Ensure that the all of spawned tasks are terminated and their instance
/// are destroyed before `Self::drop`.
#[derive(Debug)]
pub struct App<E: IsAppEndpoint> {
    endpoint: Lift<E>,
}

impl<E> App<E>
where
    E: IsAppEndpoint,
{
    /// Create a new `App` from the specified endpoint.
    pub fn new(endpoint: E) -> App<E> {
        App {
            endpoint: Lift(endpoint),
        }
    }
}

impl<E> NewService for App<E>
where
    E: IsAppEndpoint,
{
    type Request = Request<Body>;
    type Response = Response<AppPayload>;
    type Error = Never;
    type Service = AppService<'static, Lift<E>>;
    type InitError = Never;
    type Future = future::FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        // This unsafe code assumes that the lifetime of `&self` is always
        // longer than the generated future.
        let endpoint = unsafe { &*(&self.endpoint as *const _) };
        future::ok(AppService { endpoint })
    }
}

pub(crate) mod app_service {
    use std::fmt;
    use std::mem;

    use futures::{Async, Future, Poll};
    use http::header;
    use http::header::HeaderValue;
    use http::{Request, Response};
    use hyper::body::Body;
    use tower_service::Service;

    use endpoint::context::{ApplyContext, TaskContext};
    use endpoint::{with_set_cx, Cursor, Endpoint};
    use error::{Error, Never};
    use input::{Input, ReqBody};
    use output::{Output, OutputContext};

    use super::AppPayload;

    #[derive(Debug)]
    pub struct AppService<'e, E: Endpoint<'e>> {
        pub(super) endpoint: &'e E,
    }

    impl<'e, E> AppService<'e, E>
    where
        E: Endpoint<'e>,
    {
        pub(crate) fn new(endpoint: &'e E) -> AppService<'e, E> {
            AppService { endpoint }
        }

        pub(crate) fn dispatch(&self, request: Request<ReqBody>) -> AppFuture<'e, E> {
            AppFuture {
                endpoint: self.endpoint,
                state: State::Start(request),
            }
        }
    }

    impl<'e, E> Service for AppService<'e, E>
    where
        E: Endpoint<'e>,
        E::Output: Output,
    {
        type Request = Request<Body>;
        type Response = Response<AppPayload>;
        type Error = Never;
        type Future = AppFuture<'e, E>;

        fn poll_ready(&mut self) -> Poll<(), Self::Error> {
            Ok(Async::Ready(()))
        }

        fn call(&mut self, request: Self::Request) -> Self::Future {
            self.dispatch(request.map(ReqBody::new))
        }
    }

    #[derive(Debug)]
    pub struct AppFuture<'e, E: Endpoint<'e>> {
        endpoint: &'e E,
        state: State<'e, E>,
    }

    enum State<'a, E: Endpoint<'a>> {
        Start(Request<ReqBody>),
        InFlight(Input, E::Future, Cursor),
        Done(Input),
        Gone,
    }

    impl<'a, E: Endpoint<'a>> fmt::Debug for State<'a, E> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                State::Start(ref request) => {
                    f.debug_struct("Start").field("request", request).finish()
                }
                State::InFlight(ref input, _, ref cursor) => f
                    .debug_struct("InFlight")
                    .field("input", input)
                    .field("cursor", cursor)
                    .finish(),
                State::Done(ref input) => f.debug_struct("Done").field("input", input).finish(),
                State::Gone => f.debug_struct("Gone").finish(),
            }
        }
    }

    impl<'e, E> AppFuture<'e, E>
    where
        E: Endpoint<'e>,
    {
        pub(crate) fn poll_endpoint(&mut self) -> Poll<E::Output, Error> {
            loop {
                let result = match self.state {
                    State::Start(..) => None,
                    State::InFlight(ref mut input, ref mut f, ref mut cursor) => {
                        let mut tcx = TaskContext::new(input, cursor);
                        match with_set_cx(&mut tcx, || f.poll()) {
                            Ok(Async::NotReady) => return Ok(Async::NotReady),
                            Ok(Async::Ready(ok)) => Some(Ok(ok)),
                            Err(err) => Some(Err(err)),
                        }
                    }
                    State::Done(..) | State::Gone => panic!("cannot poll AppServiceFuture twice"),
                };

                match (mem::replace(&mut self.state, State::Gone), result) {
                    (State::Start(request), None) => {
                        let mut input = Input::new(request);
                        let mut cursor = Cursor::default();
                        match {
                            let mut ecx = ApplyContext::new(&mut input, &mut cursor);
                            self.endpoint.apply(&mut ecx)
                        } {
                            Ok(future) => self.state = State::InFlight(input, future, cursor),
                            Err(err) => {
                                self.state = State::Done(input);
                                return Err(err.into());
                            }
                        }
                    }
                    (State::InFlight(input, ..), Some(result)) => {
                        self.state = State::Done(input);
                        return result.map(Async::Ready);
                    }
                    _ => unreachable!("unexpected state"),
                }
            }
        }

        pub(crate) fn poll_output(&mut self) -> Poll<Response<<E::Output as Output>::Body>, Error>
        where
            E::Output: Output,
        {
            let output = try_ready!(self.poll_endpoint());
            match self.state {
                State::Done(ref mut input) => {
                    let mut cx = OutputContext::new(input);
                    output
                        .respond(&mut cx)
                        .map(|res| Async::Ready(res))
                        .map_err(Into::into)
                }
                _ => unreachable!("unexpected condition"),
            }
        }
    }

    impl<'e, E> Future for AppFuture<'e, E>
    where
        E: Endpoint<'e>,
        E::Output: Output,
    {
        type Item = Response<AppPayload>;
        type Error = Never;

        fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
            let output = match self.poll_output() {
                Ok(Async::Ready(item)) => Ok(item),
                Ok(Async::NotReady) => return Ok(Async::NotReady),
                Err(err) => Err(err),
            };

            match mem::replace(&mut self.state, State::Gone) {
                State::Done(input) => {
                    let mut response = input.finalize(output).map(AppPayload::new);
                    response
                        .headers_mut()
                        .entry(header::SERVER)
                        .unwrap()
                        .or_insert(HeaderValue::from_static(concat!(
                            "finchers/",
                            env!("CARGO_PKG_VERSION")
                        )));
                    Ok(Async::Ready(response))
                }
                _ => unreachable!("unexpected condition"),
            }
        }
    }
}

mod app_payload {
    use std::error;
    use std::fmt;
    use std::io;

    use bytes::Buf;
    use either::Either;
    use futures::{Async, Poll, Stream};
    use http::header::HeaderMap;
    use hyper::body::Payload;

    use output::body::ResBody;

    type AppPayloadData = Either<io::Cursor<String>, Box<dyn Buf + Send + 'static>>;
    type BoxedData = Box<dyn Buf + Send + 'static>;
    type BoxedError = Box<dyn error::Error + Send + Sync + 'static>;
    type BoxedPayload = Box<dyn Payload<Data = BoxedData, Error = BoxedError>>;

    /// A payload which will be returned from services generated by `App`.
    pub struct AppPayload {
        inner: Either<Option<String>, BoxedPayload>,
    }

    impl fmt::Debug for AppPayload {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self.inner {
                Either::Left(ref err) => f.debug_tuple("Err").field(err).finish(),
                Either::Right(..) => f.debug_tuple("Ok").finish(),
            }
        }
    }

    impl AppPayload {
        pub(super) fn new<T>(body: Either<String, T>) -> Self
        where
            T: ResBody,
        {
            match body {
                Either::Left(message) => Self::err(message),
                Either::Right(body) => Self::ok(body),
            }
        }

        fn err(message: String) -> Self {
            AppPayload {
                inner: Either::Left(Some(message)),
            }
        }

        fn ok<T: ResBody>(body: T) -> Self {
            struct Inner<T: Payload>(T);

            impl<T: Payload> Payload for Inner<T> {
                type Data = BoxedData;
                type Error = BoxedError;

                #[inline]
                fn poll_data(&mut self) -> Poll<Option<Self::Data>, Self::Error> {
                    self.0
                        .poll_data()
                        .map(|x| x.map(|data_opt| data_opt.map(|data| Box::new(data) as BoxedData)))
                        .map_err(Into::into)
                }

                fn poll_trailers(&mut self) -> Poll<Option<HeaderMap>, Self::Error> {
                    self.0.poll_trailers().map_err(Into::into)
                }

                fn is_end_stream(&self) -> bool {
                    self.0.is_end_stream()
                }

                fn content_length(&self) -> Option<u64> {
                    self.0.content_length()
                }
            }

            AppPayload {
                inner: Either::Right(Box::new(Inner(body.into_payload()))),
            }
        }
    }

    impl Payload for AppPayload {
        type Data = AppPayloadData;
        type Error = BoxedError;

        #[inline]
        fn poll_data(&mut self) -> Poll<Option<Self::Data>, Self::Error> {
            match self.inner {
                Either::Left(ref mut message) => message
                    .take()
                    .map(|message| Ok(Async::Ready(Some(Either::Left(io::Cursor::new(message))))))
                    .expect("The payload has already polled"),
                Either::Right(ref mut payload) => payload
                    .poll_data()
                    .map(|x| x.map(|data_opt| data_opt.map(Either::Right))),
            }
        }

        fn poll_trailers(&mut self) -> Poll<Option<HeaderMap>, Self::Error> {
            match self.inner {
                Either::Left(..) => Ok(Async::Ready(None)),
                Either::Right(ref mut payload) => payload.poll_trailers(),
            }
        }

        fn is_end_stream(&self) -> bool {
            match self.inner {
                Either::Left(ref msg) => msg.is_none(),
                Either::Right(ref payload) => payload.is_end_stream(),
            }
        }

        fn content_length(&self) -> Option<u64> {
            match self.inner {
                Either::Left(ref msg) => msg.as_ref().map(|msg| msg.len() as u64),
                Either::Right(ref payload) => payload.content_length(),
            }
        }
    }

    impl Stream for AppPayload {
        type Item = AppPayloadData;
        type Error = BoxedError;

        fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
            self.poll_data()
        }
    }

    #[cfg(feature = "tower-web")]
    mod imp_buf_stream_for_app_payload {
        use super::*;

        use futures::Poll;
        use hyper::body::Payload;
        use tower_web::util::buf_stream::size_hint;
        use tower_web::util::BufStream;

        impl BufStream for AppPayload {
            type Item = AppPayloadData;
            type Error = BoxedError;

            fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
                self.poll_data()
            }

            fn size_hint(&self) -> size_hint::SizeHint {
                let mut builder = size_hint::Builder::new();
                if let Some(length) = self.content_length() {
                    if length < usize::max_value() as u64 {
                        let length = length as usize;
                        builder.lower(length).upper(length);
                    } else {
                        builder.lower(usize::max_value());
                    }
                }
                builder.build()
            }
        }
    }
}

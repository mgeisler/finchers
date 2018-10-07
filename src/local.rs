//! Utilities for testing endpoints.
//!
//! # Example
//!
//! ```
//! # #[macro_use]
//! # extern crate finchers;
//! # use finchers::prelude::*;
//! # use finchers::local;
//! #
//! # fn main() {
//! // impl Endpoint<Output = (u32, String)>
//! let endpoint = path!(@post / u32 /)
//!     .and(endpoints::body::text());
//!
//! const MSG: &str = "The quick brown fox jumps over the lazy dog";
//!
//! let output = local::post("/42").body(MSG).apply(&endpoint);
//! match output {
//!     Ok((ref id, ref body)) => {
//!         assert_eq!(*id, 42);
//!         assert_eq!(body, MSG);
//!     }
//!     Err(..) => panic!("assertion failed"),
//! }
//! # }
//! ```
//!
//! ```
//! # #[macro_use]
//! # extern crate finchers;
//! # use finchers::prelude::*;
//! # use finchers::local;
//! #
//! # fn main() {
//! let endpoint = path!(@put / "posts" / u32 /)
//!     .and(endpoints::body::text())
//!     .map(|id: u32, body: String| {
//!         format!("update a post (id = {}): {}", id, body)
//!     });
//!
//! let response = local::put("/posts/42")
//!     .body("Yee.")
//!     .respond(&endpoint);
//!
//! assert_eq!(
//!     response.status().as_u16(),
//!     200
//! );
//! assert_eq!(
//!     response.headers()
//!         .get("content-type")
//!         .map(|h| h.as_bytes()),
//!     Some(&b"text/plain; charset=utf-8"[..])
//! );
//! assert_eq!(
//!     response.body().to_utf8(),
//!     "update a post (id = 42): Yee."
//! );
//! # }
//! ```

#![deprecated(
    since = "0.12.3",
    note = "use the new testing falicity instead."
)]
#![allow(deprecated)]

use std::borrow::Cow;
use std::mem;

use futures::future as future01;
use futures::stream as stream01;
use futures::Async;
use futures::Stream as _Stream01;

use bytes::{Buf, Bytes};
use http::header::{HeaderMap, HeaderName, HeaderValue};
use http::{HttpTryFrom, Method, Request, Response, Uri};
use hyper::body::{Body, Payload};
use tokio::runtime::current_thread::Runtime;

use app::deprecated::App;
use endpoint::Endpoint;
use error::Error;
use input::ReqBody;
use output::body::ResBody as _ResBody;
use output::Output;

use rt::{with_set_runtime_mode, RuntimeMode};

fn annotate<R>(f: impl FnOnce() -> R) -> R {
    with_set_runtime_mode(RuntimeMode::CurrentThread, f)
}

macro_rules! impl_constructors {
    ($(
        $(#[$doc:meta])*
        $METHOD:ident => $name:ident,
    )*) => {$(
        $(#[$doc])*
        pub fn $name<U>(uri: U) -> LocalRequest
        where
            Uri: HttpTryFrom<U>,
        {
            (LocalRequest {
                request: Some(Request::new(ReqBody::new(Default::default()))),
            })
            .method(Method::$METHOD)
            .uri(uri)
        }
    )*};
}

impl_constructors! {
    /// Create a dummy `GET` request with given URI.
    GET => get,

    /// Create a dummy `POST` request with given URI.
    POST => post,

    /// Create a dummy `PUT` request with given URI.
    PUT => put,

    /// Create a dummy `HEAD` request with given URI.
    HEAD => head,

    /// Create a dummy `DELETE` request with given URI.
    DELETE => delete,

    /// Create a dummy `PATCH` request with given URI.
    PATCH => patch,

    /// Create a dummy `OPTIONS` request with given URI.
    OPTIONS => options,
}

/// A builder of dummy HTTP request.
#[derive(Debug)]
pub struct LocalRequest {
    request: Option<Request<ReqBody>>,
}

impl LocalRequest {
    /// Overwrite the HTTP method of this dummy request with given value.
    ///
    /// # Panics
    /// This method will panic if the parameter is invalid HTTP method.
    pub fn method<M>(mut self, method: M) -> Self
    where
        Method: HttpTryFrom<M>,
    {
        if let Some(ref mut request) = self.request {
            *request.method_mut() = Method::try_from(method).ok().unwrap();
        }
        self
    }

    /// Overwrite the URI of this dummy request with given value.
    ///
    /// # Panics
    /// This method will panic if the parameter is invalid HTTP method.
    pub fn uri<U>(mut self, uri: U) -> Self
    where
        Uri: HttpTryFrom<U>,
    {
        if let Some(ref mut request) = self.request {
            *request.uri_mut() = Uri::try_from(uri).ok().unwrap();
        }
        self
    }

    /// Append the given header entry into this dummy request.
    ///
    /// # Panics
    /// This method will panic if the given header name or value is invalid.
    pub fn header<K, V>(mut self, name: K, value: V) -> Self
    where
        HeaderName: HttpTryFrom<K>,
        HeaderValue: HttpTryFrom<V>,
    {
        if let Some(ref mut request) = self.request {
            let name = HeaderName::try_from(name).ok().unwrap();
            let value = HeaderValue::try_from(value).ok().unwrap();
            request.headers_mut().insert(name, value);
        }
        self
    }

    /// Overwrite the message body of this dummy request with given instance.
    pub fn body(mut self, body: impl Into<Body>) -> Self {
        if let Some(ref mut request) = self.request {
            mem::replace(request.body_mut(), ReqBody::new(body.into()));
        }
        self
    }

    /// Apply this dummy request to the associated endpoint and get its response.
    #[allow(deprecated)]
    pub fn apply<'e, E: Endpoint<'e>>(self, endpoint: &'e E) -> Result<E::Output, Error> {
        let LocalRequest { mut request } = self;
        let request = request.take().expect("The request has already applied");

        let app = App::new(endpoint);

        let mut future = app.dispatch_request(request);
        let future = ::futures::future::poll_fn(move || annotate(|| future.poll_output()));

        let mut rt = Runtime::new().expect("rt");
        rt.block_on(future)
    }

    #[allow(missing_docs)]
    #[allow(deprecated)]
    pub fn respond<'e, E>(self, endpoint: &'e E) -> Response<ResBody>
    where
        E: Endpoint<'e>,
        E::Output: Output,
    {
        let LocalRequest { mut request } = self;
        let request = request.take().expect("The request has already applied");

        let app = App::new(endpoint);

        let mut future = app.dispatch_request(request);
        let future = ::futures::future::poll_fn(move || annotate(|| future.poll_response()));

        let mut rt = Runtime::new().expect("rt");

        let response = rt
            .block_on(future)
            .expect("AppFuture::poll_response() never fail");
        let (parts, body) = response.into_parts();
        let mut body = body.into_payload();

        // construct ResBody
        let content_length = body.content_length();

        let data = rt
            .block_on(
                stream01::poll_fn(|| match annotate(|| body.poll_data()) {
                    Ok(Async::Ready(data)) => Ok(Async::Ready(data.map(Buf::collect))),
                    Ok(Async::NotReady) => Ok(Async::NotReady),
                    Err(err) => Err(err),
                }).collect(),
            ).expect("error during sending the response body.");

        let trailers = rt
            .block_on(future01::poll_fn(|| annotate(|| body.poll_trailers())))
            .expect("error during sending trailers.");

        let body = ResBody {
            data,
            trailers,
            content_length,
        };

        Response::from_parts(parts, body)
    }
}

#[allow(missing_docs)]
#[derive(Debug)]
pub struct ResBody {
    data: Vec<Bytes>,
    trailers: Option<HeaderMap>,
    content_length: Option<u64>,
}

#[allow(missing_docs)]
impl ResBody {
    pub fn into_chunks(self) -> Vec<Bytes> {
        self.data
    }

    pub fn is_chunked(&self) -> bool {
        self.content_length.is_none()
    }

    pub fn trailers(&self) -> Option<&HeaderMap> {
        self.trailers.as_ref()
    }

    pub fn content_length(&self) -> Option<u64> {
        self.content_length
    }

    pub fn to_bytes(&self) -> Cow<'_, [u8]> {
        match self.data.len() {
            0 => Cow::Borrowed(&[]),
            1 => Cow::Borrowed(self.data[0].as_ref()),
            _ => Cow::Owned(self.data.iter().fold(Vec::new(), |mut acc, chunk| {
                acc.extend_from_slice(&chunk);
                acc
            })),
        }
    }

    pub fn to_utf8(&self) -> Cow<'_, str> {
        match self.to_bytes() {
            Cow::Borrowed(bytes) => String::from_utf8_lossy(bytes),
            Cow::Owned(bytes) => match String::from_utf8_lossy(&bytes) {
                Cow::Borrowed(..) => Cow::Owned(unsafe { String::from_utf8_unchecked(bytes) }),
                Cow::Owned(bytes) => Cow::Owned(bytes),
            },
        }
    }
}

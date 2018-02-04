//! A combinator library for building asynchronous HTTP services.

#![doc(html_root_url = "https://docs.rs/finchers/0.10.1")]
#![deny(missing_docs)]
#![deny(missing_debug_implementations)]
#![deny(warnings)]

#[macro_use]
pub extern crate futures;
pub extern crate http;
extern crate hyper;
pub extern crate mime;
#[cfg(feature = "tls")]
extern crate native_tls;
extern crate net2;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_service;
#[cfg(feature = "tls")]
extern crate tokio_tls;

#[allow(unused_imports)]
#[macro_use]
extern crate finchers_derive;
#[doc(hidden)]
pub use finchers_derive::*;

#[macro_use]
mod macros;

pub mod application;
pub mod endpoint;
pub mod errors;
pub mod request;
pub mod response;
pub mod service;
pub mod test;

#[doc(inline)]
pub use errors::Error;

#[allow(missing_docs)]
pub mod prelude {
    pub use endpoint::Endpoint;
    pub use service::EndpointServiceExt;
    pub use test::EndpointTestExt;
}

//! Components for checking the HTTP method

use endpoint::{Endpoint, EndpointContext, IntoEndpoint};
use hyper::Method;

#[allow(missing_docs)]
#[derive(Debug, Clone)]
pub struct MatchMethod<E: Endpoint> {
    method: Method,
    endpoint: E,
}

impl<E: Endpoint> Endpoint for MatchMethod<E> {
    type Item = E::Item;
    type Result = E::Result;

    fn apply(&self, ctx: &mut EndpointContext) -> Option<Self::Result> {
        if *ctx.method() == self.method {
            self.endpoint.apply(ctx)
        } else {
            None
        }
    }
}

#[allow(missing_docs)]
pub fn method<E: IntoEndpoint>(method: Method, endpoint: E) -> MatchMethod<E::Endpoint> {
    MatchMethod {
        method,
        endpoint: endpoint.into_endpoint(),
    }
}

macro_rules! define_method {
    ($(
        ($name:ident, $method:ident, $Endpoint:ident),
    )*) => {$(
        #[allow(missing_docs)]
        pub fn $name<E: IntoEndpoint>(endpoint: E) -> $Endpoint<E::Endpoint> {
            $Endpoint {
                endpoint: endpoint.into_endpoint(),
            }
        }

        #[allow(missing_docs)]
        #[derive(Debug, Copy, Clone)]
        pub struct $Endpoint<E> {
            endpoint: E,
        }

        impl<E: Endpoint> Endpoint for $Endpoint<E> {
            type Item = E::Item;
            type Result = E::Result;

            fn apply(&self, ctx: &mut EndpointContext) -> Option<Self::Result> {
                if *ctx.method() == Method::$method {
                    self.endpoint.apply(ctx)
                } else {
                    None
                }
            }
        }
    )*};
}

define_method! {
    (get, Get, MatchGet),
    (post, Post, MatchPost),
    (put, Put, MatchPut),
    (delete, Delete, MatchDelete),
    (head, Head, MatchHead),
    (patch, Patch, MatchPatch),
    (trace, Trace, MatchTrace),
    (connect, Connect, MatchConnect),
    (options, Options, MatchOptions),
}

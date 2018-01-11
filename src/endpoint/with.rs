#![allow(missing_docs)]

use endpoint::{Endpoint, EndpointContext, IntoEndpoint};

pub fn with<E1, E2, A, B, C>(e1: E1, e2: E2) -> With<E1::Endpoint, E2::Endpoint>
where
    E1: IntoEndpoint<A, B>,
    E2: IntoEndpoint<C, B>,
{
    With {
        e1: e1.into_endpoint(),
        e2: e2.into_endpoint(),
    }
}

#[derive(Debug, Copy, Clone)]
pub struct With<E1, E2>
where
    E1: Endpoint,
    E2: Endpoint<Error = E1::Error>,
{
    e1: E1,
    e2: E2,
}

impl<E1, E2> Endpoint for With<E1, E2>
where
    E1: Endpoint,
    E2: Endpoint<Error = E1::Error>,
{
    type Item = E2::Item;
    type Error = E2::Error;
    type Task = E2::Task;

    fn apply(&self, ctx: &mut EndpointContext) -> Option<Self::Task> {
        let _f1 = try_opt!(self.e1.apply(ctx));
        let f2 = try_opt!(self.e2.apply(ctx));
        Some(f2)
    }
}
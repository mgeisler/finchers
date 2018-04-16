use super::maybe_done::MaybeDone;
use finchers_core::endpoint::task::{self, Async, PollTask, Task};
use finchers_core::endpoint::{Context, Endpoint, IntoEndpoint};
use std::mem;

pub fn all<I>(iter: I) -> All<<I::Item as IntoEndpoint>::Endpoint>
where
    I: IntoIterator,
    I::Item: IntoEndpoint,
    <I::Item as IntoEndpoint>::Item: Send,
{
    All {
        inner: iter.into_iter().map(IntoEndpoint::into_endpoint).collect(),
    }
}

#[derive(Clone, Debug)]
pub struct All<E> {
    inner: Vec<E>,
}

impl<E> Endpoint for All<E>
where
    E: Endpoint,
    E::Item: Send,
{
    type Item = Vec<E::Item>;
    type Task = AllTask<E::Task>;

    fn apply(&self, cx: &mut Context) -> Option<Self::Task> {
        let mut elems = Vec::with_capacity(self.inner.len());
        for e in &self.inner {
            let f = e.apply(cx)?;
            elems.push(MaybeDone::Pending(f));
        }
        Some(AllTask { elems })
    }
}

pub struct AllTask<T: Task> {
    elems: Vec<MaybeDone<T>>,
}

impl<T: Task> Task for AllTask<T> {
    type Output = Vec<T::Output>;

    fn poll_task(&mut self, cx: &mut task::Context) -> PollTask<Self::Output> {
        let mut all_done = true;
        for i in 0..self.elems.len() {
            match self.elems[i].poll_done(cx) {
                Ok(done) => all_done = all_done & done,
                Err(e) => {
                    self.elems = Vec::new();
                    return Err(e);
                }
            }
        }
        if all_done {
            let elems = mem::replace(&mut self.elems, Vec::new())
                .into_iter()
                .map(|mut m| m.take_item())
                .collect();
            Ok(Async::Ready(elems))
        } else {
            Ok(Async::NotReady)
        }
    }
}
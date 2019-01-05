//! Components for building endpoints which matches to a specific HTTP path.

mod encoded;
pub mod verb;

pub use self::encoded::{EncodedStr, FromEncodedStr};

use {
    crate::{
        endpoint::{
            ActionContext, //
            ApplyContext,
            Endpoint,
            EndpointAction,
            IsEndpoint,
            Oneshot,
            OneshotAction,
        },
        error::{BadRequest, Error},
    },
    futures::Poll,
    http::StatusCode,
    percent_encoding::{percent_encode, DEFAULT_ENCODE_SET},
    std::{fmt, marker::PhantomData, sync::Arc},
};

#[doc(hidden)]
#[derive(Debug)]
#[must_use = "futures does not anything unless polled."]
pub struct Matched {
    _priv: (),
}

impl<Bd> EndpointAction<Bd> for Matched {
    type Output = ();
    type Error = Error;

    #[inline]
    fn poll_action(&mut self, _: &mut ActionContext<'_, Bd>) -> Poll<Self::Output, Self::Error> {
        Ok(().into())
    }
}

#[doc(hidden)]
#[derive(Debug)]
#[must_use = "futures does not anything unless polled."]
pub struct Extracted<T>(Option<T>);

impl<T, Bd> EndpointAction<Bd> for Extracted<T> {
    type Output = (T,);
    type Error = Error;

    #[inline]
    fn poll_action(&mut self, _: &mut ActionContext<'_, Bd>) -> Poll<Self::Output, Self::Error> {
        let x = self.0.take().expect("This future has already polled");
        Ok((x,).into())
    }
}

// ==== MatchSegment =====

percent_encoding::define_encode_set! {
    /// The encode set for MatchSegment
    #[doc(hidden)]
    pub SEGMENT_ENCODE_SET = [DEFAULT_ENCODE_SET] | {'/'}
}

/// Create an endpoint which validates a path segment.
///
/// It takes a path segment from the context and check if it is equal
/// to the specified value.
pub fn segment(s: impl AsRef<str>) -> MatchSegment {
    let s = s.as_ref();
    debug_assert!(!s.is_empty());
    MatchSegment {
        encoded: Arc::new(percent_encode(s.as_bytes(), SEGMENT_ENCODE_SET).to_string()),
    }
}

#[allow(missing_docs)]
#[derive(Debug, Clone)]
pub struct MatchSegment {
    encoded: Arc<String>,
}

impl IsEndpoint for MatchSegment {}

impl<Bd> Endpoint<Bd> for MatchSegment {
    type Output = ();
    type Error = StatusCode;
    type Action = Oneshot<MatchSegmentAction>;

    fn action(&self) -> Self::Action {
        MatchSegmentAction {
            encoded: self.encoded.clone(),
        }
        .into_action()
    }
}

#[doc(hidden)]
#[allow(missing_debug_implementations)]
pub struct MatchSegmentAction {
    encoded: Arc<String>,
}

impl OneshotAction for MatchSegmentAction {
    type Output = ();
    type Error = StatusCode;

    fn apply(self, ecx: &mut ApplyContext<'_>) -> Result<Self::Output, Self::Error> {
        let s = ecx.next().ok_or_else(|| StatusCode::NOT_FOUND)?;
        if s == &*self.encoded {
            Ok(())
        } else {
            Err(StatusCode::NOT_FOUND)
        }
    }
}

// ==== MatchEos ====

/// Create an endpoint which checks if the current context is reached the end of segments.
#[inline]
pub fn eos() -> MatchEos {
    MatchEos { _priv: () }
}

#[allow(missing_docs)]
#[derive(Debug, Copy, Clone)]
pub struct MatchEos {
    _priv: (),
}

impl IsEndpoint for MatchEos {}

impl<Bd> Endpoint<Bd> for MatchEos {
    type Output = ();
    type Error = StatusCode;
    type Action = Oneshot<MatchEosAction>;

    fn action(&self) -> Self::Action {
        MatchEosAction { _priv: () }.into_action()
    }
}

#[doc(hidden)]
#[allow(missing_debug_implementations)]
pub struct MatchEosAction {
    _priv: (),
}

impl OneshotAction for MatchEosAction {
    type Output = ();
    type Error = StatusCode;

    fn apply(self, cx: &mut ApplyContext<'_>) -> Result<Self::Output, Self::Error> {
        match cx.next() {
            None => Ok(()),
            Some(..) => Err(StatusCode::NOT_FOUND),
        }
    }
}

// ==== Param ====

/// Create an endpoint which parses a path segment into the specified type.
///
/// This endpoint will skip the current request
/// if the segments is empty or the conversion is failed.
#[inline]
pub fn param<T>() -> Param<T>
where
    T: FromEncodedStr,
{
    Param {
        _marker: PhantomData,
    }
}

#[allow(missing_docs)]
pub struct Param<T> {
    _marker: PhantomData<fn() -> T>,
}

impl<T> Copy for Param<T> {}

impl<T> Clone for Param<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> fmt::Debug for Param<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Param").finish()
    }
}

impl<T: FromEncodedStr> IsEndpoint for Param<T> {}

impl<T, Bd> Endpoint<Bd> for Param<T>
where
    T: FromEncodedStr,
{
    type Output = (T,);
    type Error = Error;
    type Action = Oneshot<ParamAction<T>>;

    fn action(&self) -> Self::Action {
        ParamAction {
            _marker: PhantomData,
        }
        .into_action()
    }
}

#[doc(hidden)]
#[allow(missing_debug_implementations)]
pub struct ParamAction<T> {
    _marker: PhantomData<fn() -> T>,
}

impl<T> OneshotAction for ParamAction<T>
where
    T: FromEncodedStr,
{
    type Output = (T,);
    type Error = Error;

    fn apply(self, cx: &mut ApplyContext<'_>) -> Result<Self::Output, Self::Error> {
        let s = cx.next().ok_or_else(|| StatusCode::NOT_FOUND)?;
        let x = T::from_encoded_str(s).map_err(BadRequest::from)?;
        Ok((x,))
    }
}

// ==== Remains ====

/// Create an endpoint which parses the remaining path segments into the specified type.
///
/// This endpoint will skip the current request if the conversion is failed.
#[inline]
pub fn remains<T>() -> Remains<T>
where
    T: FromEncodedStr,
{
    Remains {
        _marker: PhantomData,
    }
}

#[allow(missing_docs)]
pub struct Remains<T> {
    _marker: PhantomData<fn() -> (T)>,
}

impl<T> Copy for Remains<T> {}

impl<T> Clone for Remains<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> fmt::Debug for Remains<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Remains").finish()
    }
}

impl<T: FromEncodedStr> IsEndpoint for Remains<T> {}

impl<T, Bd> Endpoint<Bd> for Remains<T>
where
    T: FromEncodedStr,
{
    type Output = (T,);
    type Error = Error;
    type Action = Oneshot<RemainsAction<T>>;

    fn action(&self) -> Self::Action {
        RemainsAction {
            _marker: PhantomData,
        }
        .into_action()
    }
}

#[doc(hidden)]
#[allow(missing_debug_implementations)]
pub struct RemainsAction<T> {
    _marker: PhantomData<fn() -> T>,
}

impl<T> OneshotAction for RemainsAction<T>
where
    T: FromEncodedStr,
{
    type Output = (T,);
    type Error = Error;

    fn apply(self, cx: &mut ApplyContext<'_>) -> Result<Self::Output, Self::Error> {
        let result = T::from_encoded_str(cx.remaining_path())
            .map_err(BadRequest::from)
            .map_err(Into::into);
        drop(cx.by_ref().count());
        result.map(|x| (x,))
    }
}

// /// A helper macro for creating an endpoint which matches to the specified HTTP path.
// ///
// /// # Example
// ///
// /// The following macro call
// ///
// /// ```
// /// # #[macro_use]
// /// # extern crate finchers;
// /// # fn main() {
// /// # drop(|| {
// /// path!(@get / "api" / "v1" / "posts" / i32)
// /// # });
// /// # }
// /// ```
// ///
// /// will be expanded to the following code:
// ///
// /// ```
// /// # use finchers::prelude::*;
// /// use finchers::endpoint::syntax;
// /// # fn main() {
// /// # drop(|| {
// /// syntax::verb::get()
// ///     .and("api")
// ///     .and("v1")
// ///     .and("posts")
// ///     .and(syntax::param::<i32>())
// /// # });
// /// # }
// /// ```
// #[macro_export(local_inner_macros)]
// macro_rules! path {
//     // with method
//     (@$method:ident $($t:tt)*) => (
//         $crate::endpoint::IntoEndpointExt::and(
//             $crate::endpoint::syntax::verb::$method(),
//             path_impl!(@start $($t)*)
//         )
//     );

//     // without method
//     (/ $($t:tt)*) => ( path_impl!(@start / $($t)*) );
// }

// #[doc(hidden)]
// #[macro_export(local_inner_macros)]
// macro_rules! path_impl {
//     (@start / $head:tt $(/ $tail:tt)*) => {{
//         let __p = path_impl!(@segment $head);
//         $(
//             let __p = $crate::endpoint::IntoEndpointExt::and(__p, path_impl!(@segment $tail));
//         )*
//         __p
//     }};
//     (@start / $head:tt $(/ $tail:tt)* /) => {
//         $crate::endpoint::IntoEndpointExt::and(
//             path_impl!(@start / $head $(/ $tail)*),
//             $crate::endpoint::syntax::eos(),
//         )
//     };
//     (@start /) => ( $crate::endpoint::syntax::eos() );

//     (@segment $t:ty) => ( $crate::endpoint::syntax::param::<$t>() );
//     (@segment $s:expr) => ( $crate::endpoint::IntoEndpoint::into_endpoint($s) );
// }

extern crate bytes;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate finchers;
extern crate futures;
extern crate http;
#[macro_use]
extern crate matches;
extern crate mime;
#[macro_use]
extern crate serde;

mod endpoint;
mod endpoints;

#[test]
fn smoketest() {
    use finchers::local;
    use finchers::output::status::Created;
    use finchers::output::Json;
    use finchers::prelude::*;

    let endpoint = path!(@get / "api" / "v1" / "posts" / u32).map(|id: u32| Created(Json(id)));

    let response = local::get("/api/v1/posts/42").respond(&endpoint);
    assert_eq!(response.status().as_u16(), 201);
    assert_eq!(
        response.headers().get("content-type").map(|h| h.as_bytes()),
        Some(&b"application/json"[..])
    );
    assert_eq!(response.body().to_utf8(), "42");
}

#[cfg(feature = "rt")]
#[test]
fn smoketest_new_runtime() {
    use finchers::prelude::*;
    drop(|| finchers::rt::launch(endpoint::cloned("Hello")).serve("127.0.0.1:4000"))
}

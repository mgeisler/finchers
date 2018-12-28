use finchers::error::bad_request;
use finchers::prelude::*;
use finchers::test;
use futures::future;

#[test]
fn test_and_then_1() {
    let mut runner = test::runner(endpoint::cloned("Foo").and_then(|_| future::ok("Bar")));
    assert_matches!(
        runner.apply("/"),
        Ok(s) if s == "Bar"
    )
}

#[test]
fn test_and_then_2() {
    let mut runner = test::runner(
        endpoint::cloned("Foo").and_then(|_| future::err::<(), _>(bad_request("Bar"))),
    );
    assert_matches!(
        runner.apply("/"),
        Err(ref e) if e.status_code().as_u16() == 400
    )
}
extern crate finchers;
extern crate finchers_tungstenite;
extern crate futures;
extern crate tokio_executor;
#[macro_use]
extern crate matches;

use futures::prelude::*;
use tokio_executor::{Executor, SpawnError};

use finchers::local;
use finchers::prelude::*;
use finchers_tungstenite::Ws;

use std::cell::Cell;
use std::rc::Rc;

pub struct MockExecutor {
    spawned: Rc<Cell<bool>>,
}

impl Executor for MockExecutor {
    fn spawn(
        &mut self,
        future: Box<dyn Future<Item = (), Error = ()> + Send>,
    ) -> Result<(), SpawnError> {
        drop(future);
        self.spawned.set(true);
        Ok(())
    }
}

#[test]
fn test_handshake() {
    let spawned = Rc::new(Cell::new(false));

    let endpoint = finchers_tungstenite::ws().map({
        let spawned = spawned.clone();
        move |ws: Ws| {
            ws.executor(MockExecutor {
                spawned: spawned.clone(),
            }).on_upgrade(|stream| {
                drop(stream);
                futures::future::ok(())
            })
        }
    });

    let response = local::get("/")
        .header("host", "localhost:4000")
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .respond(&endpoint);

    assert_eq!(spawned.get(), true);

    assert_eq!(response.status().as_u16(), 101);
    assert_eq!(response.body().to_utf8(), "");
    assert_matches!(
        response.headers().get("connection"),
        Some(h) if h.to_str().unwrap().to_lowercase() == "upgrade"
    );
    assert_matches!(
        response.headers().get("upgrade"),
        Some(h) if h.to_str().unwrap().to_lowercase() == "websocket"
    );
    assert_matches!(
        response.headers().get("sec-websocket-accept"),
        Some(h) if h == "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
    );
}

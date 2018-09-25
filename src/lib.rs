//! WebSocket support for Finchers based on tungstenite

// master
#![doc(html_root_url = "https://finchers-rs.github.io/finchers-tungstenite")]
// released
//#![doc(html_root_url = "https://docs.rs/finchers-tungstenite/0.1.0-alpha.1")]
#![warn(
    missing_docs,
    missing_debug_implementations,
    nonstandard_style,
    rust_2018_idioms,
    unused,
)]
//#![warn(rust_2018_compatibility)]
#![cfg_attr(feature = "strict", deny(warnings))]
#![cfg_attr(feature = "strict", doc(test(attr(deny(warnings)))))]

extern crate base64;
#[macro_use]
extern crate failure;
extern crate finchers;
extern crate futures;
extern crate http;
extern crate hyper;
extern crate sha1;
extern crate tokio_executor;
extern crate tokio_tungstenite;
extern crate tungstenite;

mod handshake;

use finchers::endpoint::{Context, Endpoint, EndpointResult};
use finchers::output::{Output, OutputContext};

use tungstenite::protocol::{Role, WebSocketConfig};

use futures::{Async, Future, Poll};
use http::header;
use http::{Response, StatusCode};
use hyper::upgrade::Upgraded;
use tokio_executor::{DefaultExecutor, Executor};
use tokio_tungstenite::WebSocketStream;

use handshake::{accept_handshake, Accept};

pub use handshake::{HandshakeError, HandshakeErrorKind};

/// Create an endpoint which handles the WebSocket handshake request.
pub fn ws() -> WsEndpoint {
    (WsEndpoint { _priv: () }).with_output::<(Ws,)>()
}

/// An instance of `Endpoint` which handles the WebSocket handshake request.
#[derive(Debug, Copy, Clone)]
pub struct WsEndpoint {
    _priv: (),
}

impl<'a> Endpoint<'a> for WsEndpoint {
    type Output = (Ws,);
    type Future = WsFuture;

    fn apply(&'a self, _: &mut Context<'_>) -> EndpointResult<Self::Future> {
        Ok(WsFuture { _priv: () })
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct WsFuture {
    _priv: (),
}

impl Future for WsFuture {
    type Item = (Ws,);
    type Error = finchers::error::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let accept = finchers::input::with_get_cx(|input| accept_handshake(input.request()))?;
        Ok(Async::Ready((Ws {
            accept,
            config: None,
            executor: DefaultExecutor::current(),
        },)))
    }
}

/// A type representing the result of handshake handling.
///
/// The value of this type is used to build a WebSocket process
/// after upgrading the protocol.
#[derive(Debug)]
pub struct Ws<Exec: Executor = DefaultExecutor> {
    accept: Accept,
    config: Option<WebSocketConfig>,
    executor: Exec,
}

impl<Exec: Executor> Ws<Exec> {
    #[allow(missing_docs)]
    pub fn config(self, config: WebSocketConfig) -> Ws<Exec> {
        Ws {
            config: Some(config),
            ..self
        }
    }

    #[allow(missing_docs)]
    pub fn executor<T: Executor>(self, executor: T) -> Ws<T> {
        Ws {
            accept: self.accept,
            config: self.config,
            executor,
        }
    }

    /// Creates an `Output` with the specified function which constructs
    /// a `Future` representing the task after upgrading the protocol to
    /// WebSocket.
    pub fn on_upgrade<F, R>(self, upgrade: F) -> WsOutput<F, Exec>
    where
        F: FnOnce(WebSocketStream<Upgraded>) -> R + Send + 'static,
        R: Future<Item = (), Error = ()> + Send + 'static,
    {
        WsOutput {
            accept: self.accept,
            config: self.config,
            upgrade,
            executor: self.executor,
        }
    }
}

#[allow(missing_docs)]
#[derive(Debug)]
pub struct WsOutput<F, Exec> {
    accept: Accept,
    config: Option<WebSocketConfig>,
    upgrade: F,
    executor: Exec,
}

impl<F, Exec, R> Output for WsOutput<F, Exec>
where
    F: FnOnce(WebSocketStream<Upgraded>) -> R + Send + 'static,
    R: Future<Item = (), Error = ()> + Send + 'static,
    Exec: Executor,
{
    type Body = ();
    type Error = finchers::error::Error;

    fn respond(self, cx: &mut OutputContext<'_>) -> Result<Response<Self::Body>, Self::Error> {
        let WsOutput {
            accept: Accept { hash },
            config: ws_config,
            upgrade,
            mut executor,
        } = self;

        let payload = cx
            .input()
            .payload()
            .ok_or_else(|| format_err!("stolen payload"))?;

        let future = payload
            .on_upgrade()
            .map_err(|_| eprintln!("upgrade error"))
            .and_then(move |upgraded| {
                let ws_stream = WebSocketStream::from_raw_socket(upgraded, Role::Server, ws_config);
                upgrade(ws_stream)
            });
        executor
            .spawn(Box::new(future))
            .map_err(finchers::error::fail)?;

        Ok(Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .header(header::CONNECTION, "upgrade")
            .header(header::UPGRADE, "websocket")
            .header(header::SEC_WEBSOCKET_ACCEPT, &*hash)
            .body(())
            .expect("should be a valid response"))
    }
}

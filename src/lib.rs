// FIXME: remove it as soon as the rustc version used in docs.rs is updated
#![cfg_attr(finchers_inject_extern_prelude, feature(extern_prelude))]

//! WebSocket support for Finchers based on tungstenite.
//!
//! # Example
//!
//! ```
//! #[macro_use]
//! extern crate finchers;
//! extern crate finchers_tungstenite;
//! # extern crate futures;
//!
//! use finchers::prelude::*;
//! use finchers_tungstenite::{
//!   Ws,
//!   WsTransport,
//! };
//!
//! # fn main() {
//! # drop(|| {
//! let endpoint = path!(@get / "ws" /)
//!     .and(finchers_tungstenite::ws())
//!     .map(|ws: Ws| {
//!         ws.on_upgrade(|ws: WsTransport| {
//!             // ...
//! #           drop(ws);
//! #           futures::future::ok(())
//!         })
//!     });
//! #
//! # finchers::launch(endpoint).start("127.0.0.1:4000");
//! # });
//! # }
//! ```

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
#![cfg_attr(test, deny(warnings))]
#![cfg_attr(test, doc(test(attr(deny(warnings)))))]

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

pub extern crate tungstenite;

pub mod handshake;

// re-exports
pub use self::imp::{ws, Ws, WsEndpoint, WsOutput, WsTransport};
#[doc(no_inline)]
pub use tungstenite::error::Error as WsError;
#[doc(no_inline)]
pub use tungstenite::protocol::{Message, WebSocketConfig};

mod imp {
    use finchers;
    use finchers::endpoint::{ApplyContext, ApplyResult, Endpoint};
    use finchers::output::{Output, OutputContext};

    use tungstenite::protocol::{Role, WebSocketConfig};

    use futures::{Async, Future, Poll};
    use http::header;
    use http::{Response, StatusCode};
    use hyper::upgrade::Upgraded;
    use tokio_executor::{DefaultExecutor, Executor};
    use tokio_tungstenite::WebSocketStream;

    use handshake::{handshake, Accept};

    #[allow(missing_docs)]
    pub type WsTransport = WebSocketStream<Upgraded>;

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

        fn apply(&'a self, _: &mut ApplyContext<'_>) -> ApplyResult<Self::Future> {
            Ok(WsFuture { _priv: () })
        }
    }

    #[derive(Debug)]
    pub struct WsFuture {
        _priv: (),
    }

    impl Future for WsFuture {
        type Item = (Ws,);
        type Error = finchers::error::Error;

        fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
            let accept = finchers::endpoint::with_get_cx(|cx| handshake(cx.request()))?;
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
        /// Sets the configuration for upgraded WebSocket connection.
        pub fn config(self, config: WebSocketConfig) -> Ws<Exec> {
            Ws {
                config: Some(config),
                ..self
            }
        }

        /// Sets the instance of `Executor` at spawning the task which handles the upgraded
        /// WebSocket connection.
        ///
        /// By default, it is set to Tokio's `DefaultExecutor`.
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
            F: FnOnce(WsTransport) -> R + Send + 'static,
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

    /// The type representing an `Output` to be converted into an handshake response.
    ///
    /// The value will spawn a task which handles the upgraded WebSocket connection
    /// by the specified task executor, at converting it into an HTTP response.
    #[derive(Debug)]
    pub struct WsOutput<F, Exec> {
        accept: Accept,
        config: Option<WebSocketConfig>,
        upgrade: F,
        executor: Exec,
    }

    impl<F, Exec, R> Output for WsOutput<F, Exec>
    where
        F: FnOnce(WsTransport) -> R + Send + 'static,
        R: Future<Item = (), Error = ()> + Send + 'static,
        Exec: Executor,
    {
        type Body = ();
        type Error = finchers::error::Error;

        fn respond(self, cx: &mut OutputContext<'_>) -> Result<Response<Self::Body>, Self::Error> {
            let WsOutput {
                accept: Accept { hash, .. },
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
                    let ws_stream =
                        WebSocketStream::from_raw_socket(upgraded, Role::Server, ws_config);
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
}

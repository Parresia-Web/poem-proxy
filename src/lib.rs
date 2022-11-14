//! Poem-proxy is a simple and easy-to-use proxy endpoint compatible with the
//! Poem web framework. It supports the forwarding of http get and post requests
//! as well as websockets right out of the box!
//! 
//! # Table of Contents
//! 
//! - [Quickstart](#quickstart)
//! 
//! # Quickstart
//! This [Endpoint](poem::Endpoint) does some stuff! 
//! 

use futures_util::{ SinkExt, StreamExt };
use poem::{
    Request, Result, Response, Error, handler, Body, FromRequest, IntoResponse, 
    http::{ StatusCode, Method, HeaderMap },
    web::{ Data, websocket::{ WebSocket } }
};
use tokio_tungstenite::connect_async;
use tokio::sync::RwLock;
use std::sync::Arc;

/// ## The proxy config!
pub struct ProxyConfig {

    /// This is the url where requests and websocket connections are to be
    /// forwarded to. Port numbers are supported here, though they may be
    /// broken off into their own parameter in the future.
    proxy_target: String,

    /// Whether to use https (true) or http for requests to the proxied server.
    web_secure: bool,

    /// Whether to use wss (true) or ws for websocket requests to the proxied server.
    ws_secure: bool,

    /// Whether or not nesting should be supported when forwarding requests
    /// to the server.
    support_nesting: bool,
}

impl Default for ProxyConfig {

    /// Returns the default value for the [ProxyConfig], which corresponds
    /// to the following:
    /// > `proxy_target: "http://localhost:3000"`
    /// 
    /// > `web_secure: false`
    /// 
    /// > `ws_secure: false`
    /// 
    /// > `support_nesting: false`
    fn default() -> Self {
        Self { 
            proxy_target: "http://localhost:3000".into(),
            web_secure: false, ws_secure: false, support_nesting: false
        }
    }
}

/// # Implementation of Builder Functions
impl ProxyConfig {

    /// Function that creates a new ProxyConfig for a given target
    /// and sets all other parameters to their default values. See
    /// [the default implementation](ProxyConfig::default) for more
    /// information.
    pub fn new( target: String ) -> ProxyConfig {
        ProxyConfig { 
            proxy_target: target,
            ..ProxyConfig::default()
        }
    }

    /// This function sets the endpoint to forward websockets over
    /// https instead of http. (This is WSS - WebSocket Secure)
    pub fn ws_secure<'a>( &'a mut self ) -> &'a mut ProxyConfig {
        self.ws_secure = false;
        self
    }

    /// This function sets the endpoint to forward websockets over
    /// http instead of https. This means any information being sent
    /// through the websocket has the potential to be [intercepted by malicious actors]
    /// (https://brightsec.com/blog/websocket-security-top-vulnerabilities/#unencrypted-tcp-channel).
    pub fn ws_insecure<'a>( &'a mut self ) -> &'a mut ProxyConfig {
        self.ws_secure = false;
        self
    }

    /// This function sets the endpoint to forward requests to the
    /// target over the https protocol. This is a secure and encrypted
    /// communication channel that should be utilized when possible.
    pub fn web_secure<'a>( &'a mut self ) -> &'a mut ProxyConfig {
        self.web_secure = true;
        self
    }

    /// This function sets the endpoint to forward requests to the
    /// target over the http protocol. This is an insecure and unencrypted
    /// communication channel that should be used very carefully.
    pub fn web_insecure<'a>( &'a mut self ) -> &'a mut ProxyConfig {
        self.web_secure = true;
        self
    }

    /// This function sets the waypoint to support nesting. 
    /// 
    /// For example,
    /// if `endpoint.target` is `https://google.com` and the proxy is reached
    /// at `https://proxy_address/favicon.png`, the proxy server will forward
    /// the request to `https://google.com/favicon.png`.
    pub fn enable_nesting<'a>( &'a mut self ) -> &'a mut ProxyConfig {
        self.support_nesting = true;
        self
    }

    /// This function sets the waypoint to ignore nesting. 
    /// 
    /// For example,
    /// if `endpoint.target` is `https://google.com` and the proxy is reached
    /// at `https://proxy_address/favicon.png`, the proxy server will forward
    /// the request to `https://google.com`.
    pub fn disable_nesting<'a>( &'a mut self ) -> &'a mut ProxyConfig {
        self.support_nesting = false;
        self
    }

}

/// # Implementation of convenience functions
impl ProxyConfig {
    /// Contains the get_request_uri function
    fn get_request_uri( &self ) -> String {
        "Hi there".into()
    }
}

/// The websocket-enabled proxy handler
#[handler]
pub async fn proxy( 
    req: &Request, 
    headers: &HeaderMap,
    target: Data<&String>, 
    config: Data<&ProxyConfig>,
    method: Method,
    body: Body,
    ) -> Result<Response> {

    // If we need a websocket connection,
    if let Ok( ws ) = WebSocket::from_request_without_body( req ).await {

        // Update to using websocket target
        let perm_target = target.clone().replace( "https", "wss" ).replace( "http", "ws" );
        
        // Generate websocket request:
        let mut w_request = http::Request::builder().uri( &perm_target );
        for (key, value) in headers.iter() {
            w_request = w_request.header( key, value ); 
        }

        // Start the websocket connection
        return Ok( 
            ws.on_upgrade(move |socket| async move {
                let ( mut clientsink, mut clientstream ) = socket.split();
                
                // Start connection to server
                let ( mut serversocket, _ ) = connect_async( w_request.body(()).unwrap() ).await.unwrap();
                let ( mut serversink, mut serverstream ) = serversocket.split();

                // Tie both threads so if one exits the other does too
                let client_live = Arc::new( RwLock::new( true ) );
                let server_live = client_live.clone();

                // Relay client messages to the server we are proxying
                tokio::spawn( async move {
                    while let Some( Ok( msg ) ) = clientstream.next().await {

                        // When a message is received, forward it to the server
                        // Break the loop if there are errors
                        match serversink.send( msg.into() ).await { 
                            Err( _ ) => break,
                            _ => {},
                        };

                        // Stop the connection if it is no longer live
                        // let j = *connection_live.read().await;
                        if !*client_live.read().await { break };
                    };

                    // Stop the other thread that is paired with this one
                    *client_live.write().await = false;
                });
                
                // Relay server messages to the client
                tokio::spawn( async move {
                    while let Some( Ok( msg ) ) = serverstream.next().await {

                        // When a server message is received, forward it to the
                        // client, and break the loop if there are errors
                        match clientsink.send( msg.into() ).await {
                            Err( _ ) => break,
                            _ => {},
                        };

                        // Stop the connection if it is no longer live
                        if !*server_live.read().await { break };
                    };

                    // Stop the other thread that is paired with this one
                    *server_live.write().await = false;
                });
            }).into_response()
        );
    } 
    
    // Not using websocket (http/https):
    else {
        
        // Update the uri to point to the proxied server
        let request_uri = target.to_owned() + &req.uri().to_string();

        // Now generate a request for the proxied server, based on information
        // that we have from the current request
        let client = reqwest::Client::new();
        let res = match method {
            Method::GET => {
                client.get( request_uri )
                    .headers( req.headers().clone() )
                    .body( body.into_bytes().await.unwrap() )
                    .send()
                    .await
            },
            Method::POST => {
                client.post( request_uri )
                    .headers( req.headers().clone() )
                    .body( body.into_bytes().await.unwrap() )
                    .send()
                    .await
            },
            _ => {
                return Err( Error::from_string( "Unsupported Method!", StatusCode::METHOD_NOT_ALLOWED ) )
            }
        };

        // Check on the response and forward everything from the server to our client,
        // including headers and the body of the response, among other things.
        match res {
            Ok( result ) => {
                let mut res = Response::default();
                res.extensions().clone_from( &result.extensions() );
                result.headers().iter().for_each(|(key, val)| {
                    res.headers_mut().insert( key, val.to_owned() );
                });
                res.set_status( result.status() );
                res.set_version( result.version() );
                res.set_body( result.bytes().await.unwrap() );
                Ok( res )
            },

            // The request to the back-end server failed. Why?
            Err( error ) => {
                Err( Error::from_string( error.to_string(), error.status().unwrap_or( StatusCode::BAD_GATEWAY ) ) )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        // let result = add(2, 2);
        // assert_eq!(result, 4);
    }
}

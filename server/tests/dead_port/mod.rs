//! An address that refuses connections and that no other test can be given.
//!
//! The obvious way to get a refused connection is to bind a port, read its number
//! and close the socket. That races: tests in one process run in parallel and each
//! mock server asks the OS for a free port, so one of them can be handed the port
//! just closed and answer the request that was meant to fail (it did, in CI, and a
//! "connection failure" test saw a real verdict).
//!
//! This binds the port and keeps the socket for as long as the guard lives, but
//! never calls `listen`: the OS will not hand the port to anyone else, and a
//! connection to it is refused.
#![allow(dead_code)]

use std::net::SocketAddr;

use socket2::{Domain, Socket, Type};

pub struct DeadPort {
    pub addr: SocketAddr,
    // Held, not used: dropping it frees the port.
    _socket: Socket,
}

impl DeadPort {
    pub fn new() -> Self {
        let socket = Socket::new(Domain::IPV4, Type::STREAM, None).expect("create a socket");
        let any: SocketAddr = "127.0.0.1:0".parse().expect("loopback address");
        socket.bind(&any.into()).expect("bind a loopback port");
        let addr = socket
            .local_addr()
            .expect("read the bound address")
            .as_socket()
            .expect("an IP address");
        Self {
            addr,
            _socket: socket,
        }
    }
}

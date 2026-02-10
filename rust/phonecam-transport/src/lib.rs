#![allow(dead_code)]

pub mod client;
pub mod server;
pub mod state;

pub use client::{PhoneCamClient, TransportConnection, TransportError};
pub use server::PhoneCamServer;
pub use state::ConnectionState;

#[cfg(test)]
mod tests;

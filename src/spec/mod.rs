use spec_derive::client;

pub mod auth;
pub mod library;

#[client]
pub struct Api;

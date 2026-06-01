pub mod schema;
pub mod shared;

// todo: this is bad, setup envy config struct w/ port numbers and derive the address
// from the port where needed instead
pub const WS_PORT: i32 = 8000;
pub const SERVER_URL: &str = "127.0.0.1:3000";
// todo: this is horrid
pub const SERVER_URL_HTTP: &str = "http://127.0.0.1:3000";

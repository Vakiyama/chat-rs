// the goal:
//
// // This one block defines:
// - the HTTP method and path (once)
// - the request/response types (once, from the trait)
// - the server implementation
// - the generated axum router
// - the generated reqwest client
//
// #[derive_client_handler]
// impl RoomsApi for RoomsService {
//     #[http(POST, "/rooms")]
//     async fn create_room(&self, #[body] req: CreateRoomRequest) -> Result<RoomResponse, ApiError> {
//         Ok(RoomResponse { id: 1, name: req.name })
//     }
//
//     #[]
//
//     #[http(GET, "/rooms/:id")]
//     async fn get_room(&self, #[path] id: u64) -> Result<RoomResponse, ApiError> {
//         Ok(RoomResponse { id, name: "general".into() })
//     }
// }
//
//
// we need to have:
// a macro that, given a impl _ for MainStruct, generates:
// 1.
// an axum router, ideally in the order given. we could add a #[middleware] macro to define layers
// as well, in the order given.
// we need to define layers and nested routers as well with this setup, allowing us the same
// composability of axum.
// we also probably need a way to define what state would be given for this router.
//
//
// 2.
// a reqwest client, sharing the api interface. we can encode/decode everything as json for now for
// simplicity, allowing axum extractors to handle the incoming.

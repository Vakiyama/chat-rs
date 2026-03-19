use proc_macro::TokenStream;
use quote::quote;
use syn::Item;

mod generate;

#[proc_macro_attribute]
pub fn generate(_metadata: TokenStream, input: TokenStream) -> TokenStream {
  let item: Item = syn::parse(input).expect("failed to parse input");
  if let Item::Impl(item_impl) = item {
    generate::handle_trait(item_impl).into()
  } else {
    let error = syn::Error::new_spanned(
      item,
      "This attribute can only be applied to trait implementations",
    )
    .to_compile_error();

    TokenStream::from(error)
  }
}

#[proc_macro_attribute]
pub fn client(_metadata: TokenStream, input: TokenStream) -> TokenStream {
  let item: Item = syn::parse(input).expect("Failed to parse input");
  if let Item::Struct(item_struct) = item {
    let ident = &item_struct.ident;
    let vis = &item_struct.vis;

    let struct_tokens = quote! {
      #vis struct #ident {
          pub inner: reqwest::Client,
          pub base_url: std::sync::Arc<String>,
      }
    };

    let new_impl = quote! {
        impl #ident {
            #vis fn new(base_url: impl Into<String>) -> Self {
                Self {
                    inner: reqwest::Client::new(),
                    base_url: std::sync::Arc::new(base_url.into())
                }
            }
        }
    };

    let tokens = quote! {
        #struct_tokens

        #new_impl
    };

    eprintln!("{tokens}");

    tokens.into()
  } else {
    syn::Error::new_spanned(item, "This attribute can only be applied to a struct")
      .to_compile_error()
      .into()
  }
}

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
//     #[layer] // name is abitrary "auth" could be anything here
//     fn auth() {
//      //   this is an expression that returns the arguments to .layer
//   tower::ServiceBuilder::new().layer(AsyncRequireAuthorizationLayer::new(JWTAuthorized {
//     key: JWTKey { key: key.into() }.into(),
//   })),
//     }
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
//
// ideally, we'd support more than just json wrapped responses
// we can try and use trait bounds on the responses
// axum expects reponses to impl Into<axum::response>, so that side is handled
// we can add our own trait bound to go from axum::response -> T
// in the client, we call this trait method
// this gives users full control over both ends

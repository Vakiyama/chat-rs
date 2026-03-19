use darling::FromMeta;
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{Attribute, FnArg, ImplItem, ImplItemFn, ItemImpl, LitStr, Token, parse::Parse};

// the actual impl:
// pub struct ItemImpl {
//     pub attrs: Vec<Attribute>,
//     pub defaultness: Option<Default>,
//     pub unsafety: Option<Unsafe>,
//     pub impl_token: Impl,
//     pub generics: Generics,
//     pub trait_: Option<(Option<Not>, Path, For)>,
//     pub self_ty: Box<Type>,
//     pub brace_token: Brace,
//     pub items: Vec<ImplItem>,
// }
//
// pub enum ImplItem {
//  Const(ImplItemConst),
//  Fn(ImplItemFn),
//  Type(ImplItemType),
//  Macro(ImplItemMacro),
//  Verbatim(TokenStream),
//}

pub fn handle_trait(item_impl: ItemImpl) -> TokenStream {
  // each item can be turned into the naked tokenstream fn using quote!
  // we can then quote! compose these later
  let items = &item_impl.items;

  // let parsed_handlers: Vec<Result<TokenStream, syn::Error>> = items
  //   .iter()
  //   .map(|impl_item: &ImplItem| parse_item_into_handler(impl_item))
  //   .collect();

  let with_http: Vec<Result<(TokenStream, Http, &syn::Ident), syn::Error>> = items
    .iter()
    .map(|impl_item: &ImplItem| parse_item_into_handler(impl_item))
    .collect();

  let routes: TokenStream = with_http
    .iter()
    .filter_map(|item| item.as_ref().ok())
    .map(|(_, http, ident): &(TokenStream, Http, &syn::Ident)| {
      let axum_method = http.method.as_axum_fn();
      let path = http.path.value();
      let stringified_ident = ident.to_string();
      let handler_ident: &syn::Ident =
        &syn::Ident::from_string(&format!("{stringified_ident}_handler")).unwrap();

      quote! {
          .route(#path, #axum_method(#handler_ident))
      }
    })
    .collect();

  let router_name = match &item_impl.trait_ {
    Some((_, path, _)) => path
      .segments
      .last()
      .map(|seg| {
        let trait_name = seg.ident.to_string();

        let mut router_name: Vec<char> = format!("{trait_name}_handler")
          .chars()
          .flat_map(|char: char| {
            if char.is_uppercase() {
              vec!['_', char]
            } else {
              vec![char]
            }
          })
          .collect();

        router_name.remove(0);

        let router_name: String = router_name.into_iter().collect();

        let router_name: syn::Ident = syn::Ident::from_string(&router_name.to_lowercase()).unwrap();

        quote! { #router_name }
      })
      .ok_or(
        syn::Error::new_spanned(&item_impl, "Failed to find type name for trait impl")
          .to_compile_error(),
      ),
    None => Err(
      syn::Error::new_spanned(&item_impl, "Failed to find type name for trait impl")
        .to_compile_error(),
    ),
  }
  .unwrap_or_else(|e| e);

  let router = quote! {
    pub fn #router_name() -> axum::Router {
        axum::Router::new()
            #routes
    }
  };

  let parsed_handlers: TokenStream = with_http
    .into_iter()
    .map(|item| {
      item
        .map(|item| item.0)
        .unwrap_or_else(|e| e.to_compile_error())
    })
    .collect();

  // we want two structures:
  // 1.
  // a router with a name based on the itemimpl,
  // the router can be pub based on the visibility of the itemimpl
  // 2.
  // the handlers, which will be used in the defined router
  // the handlers shouldn't have to pollute the name space

  // for the client, we want each macro to derive a new body for each
  // fn that handles the client side of the request, still generating that
  // same impl.
  // the api client should be derived from some struct

  let router_handlers = quote! {
      #router

      #parsed_handlers
  };

  eprintln!("{router_handlers}");

  router_handlers
}

fn parse_item_into_handler(
  impl_item: &ImplItem,
) -> Result<(TokenStream, Http, &syn::Ident), syn::Error> {
  match impl_item {
    ImplItem::Fn(impl_item_fn) => Ok((
      parse_trait_fn(impl_item_fn)?,
      find_http_attr(impl_item_fn)?,
      &impl_item_fn.sig.ident,
    )),
    _ => Err(syn::Error::new_spanned(
      impl_item,
      "Trait item must be a function.",
    )),
  }
}

pub enum HttpMethod {
  Get,
  Post,
  Put,
  Delete,
  Patch,
}

impl HttpMethod {
  pub fn as_axum_fn(&self) -> proc_macro2::TokenStream {
    match self {
      HttpMethod::Get => quote! { axum::routing::get },
      HttpMethod::Post => quote! { axum::routing::post },
      HttpMethod::Put => quote! { axum::routing::put },
      HttpMethod::Delete => quote! { axum::routing::delete },
      HttpMethod::Patch => quote! { axum::routing::patch },
    }
  }
}

impl TryFrom<syn::Path> for HttpMethod {
  type Error = syn::Error;

  fn try_from(path: syn::Path) -> Result<Self, Self::Error> {
    let ident = path.get_ident().ok_or(syn::Error::new_spanned(
      &path,
      "Expected a single identifier, like Post or Get",
    ))?;

    match ident.to_string().to_uppercase().as_str() {
      "GET" => Ok(Self::Get),
      "POST" => Ok(Self::Post),
      "PUT" => Ok(Self::Put),
      "DELETE" => Ok(Self::Delete),
      "PATCH" => Ok(Self::Patch),
      _ => Err(syn::Error::new_spanned(
        ident,
        format!("unknown HTTP method `{ident}`, expected get, post, put, delete, or patch"),
      )),
    }
  }
}

pub struct Http {
  pub method: HttpMethod, // Post, Get etc
  pub path: LitStr,       // "/rooms/:id"
}

impl Parse for Http {
  fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
    let method: syn::Path = input.parse()?;
    let method: HttpMethod = method.try_into()?;

    input.parse::<Token![,]>()?;
    let path: LitStr = input.parse()?;
    Ok(Http { method, path })
  }
}

fn parse_trait_fn(f: &ImplItemFn) -> Result<TokenStream, syn::Error> {
  // we need to rebuild the arguments from #[body] request: SomeType, #[query] ... into
  // Json(request): SomeType, Query(query): ...

  let new_params: Result<Vec<TokenStream>, syn::Error> = f
    .sig
    .inputs
    .iter()
    .filter(|arg| !matches!(arg, FnArg::Receiver(_)))
    .map(parse_sig_param)
    .collect();

  let params = new_params?;

  // at this point, we have the new params list, we can create the handler

  let fname = syn::Ident::new(
    &format!("{}_handler", &f.sig.ident.to_string()),
    Span::call_site(),
  );
  let freturn = &f.sig.output;
  let fblock = &f.block;

  let new_fn_tokens = quote! {
      #[axum::debug_handler]
      async fn #fname(#(#params),*) #freturn
          #fblock

  };

  match &f.sig.asyncness {
    Some(_) => Ok(new_fn_tokens),
    None => Err(syn::Error::new_spanned(f, "Method must be async")),
  }
}

enum Extractor {
  Json,
  Query,
  // ...
}

impl TryFrom<&syn::Path> for Extractor {
  type Error = syn::Error;

  fn try_from(value: &syn::Path) -> Result<Self, Self::Error> {
    let ident = value.get_ident().ok_or(syn::Error::new_spanned(
      value,
      "Expected a single identifier, like #[query] or #[json]",
    ))?;

    match ident.to_string().to_uppercase().as_str() {
      "JSON" => Ok(Extractor::Json),
      "QUERY" => Ok(Extractor::Query),
      _ => Err(syn::Error::new_spanned(
        ident,
        format!("unknown extractor `{ident}`, expected json, query, etc..."),
      )),
    }
  }
}

fn parse_sig_param(arg: &FnArg) -> Result<TokenStream, syn::Error> {
  match arg {
    FnArg::Receiver(_) => Ok(quote! {}), // this is the self arg
    FnArg::Typed(pat_type) => {
      let filtered: Vec<&Attribute> = pat_type
        .attrs
        .iter()
        .filter(|attr| attr.style == syn::AttrStyle::Outer)
        .collect();

      match filtered.len() {
        0 => Ok(quote! { #arg }),
        1 => {
          let attr = filtered[0];
          let extractor: Extractor = match &attr.meta {
            syn::Meta::Path(path) => path.try_into(),
            _ => Err(syn::Error::new_spanned(
              attr,
              "Attribute is not a valid extractor. Expected something like #[query]",
            )),
          }?;

          let ident_string = match pat_type.pat.as_ref() {
            syn::Pat::Ident(syn::PatIdent { ident, .. }) => Ok(ident),
            _ => Err(syn::Error::new_spanned(
              &pat_type.pat,
              "expected a simple identifier as argument name",
            )),
          }?;

          let ty = pat_type.ty.as_ref();

          // use axum::{Json, extract::State, response::IntoResponse};
          Ok(match extractor {
            Extractor::Json => quote! { axum::Json(#ident_string): axum::Json<#ty> },
            Extractor::Query => {
              quote! { axum::extract::Query(#ident_string): axum::extract::Query<#ty> }
            }
          })
        }
        _ => Err(syn::Error::new_spanned(
          arg,
          "Must have at most 1 argument annotation, such as \"#[query]\"",
        )),
      }
    }
  }
}

fn find_http_attr(f: &ImplItemFn) -> Result<Http, syn::Error> {
  let attr = f
    .attrs
    .iter()
    .find(|attr| attr.path().is_ident("http"))
    .map(Ok)
    .unwrap_or(Err(syn::Error::new_spanned(
      f,
      "Method must have an attribute like: #[http(get, \"/path\")]",
    )))?;

  attr.parse_args::<Http>().map_err(|e| {
    syn::Error::new_spanned(
      attr,
      format!("Failed to parse http attribute with error {}", e),
    )
  })
}

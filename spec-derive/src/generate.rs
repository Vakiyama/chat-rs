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
  let items = item_impl.items;

  // let parsed_handlers: Vec<Result<TokenStream, syn::Error>> = items
  //   .iter()
  //   .map(|impl_item: &ImplItem| parse_item_into_handler(impl_item))
  //   .collect();

  let parsed_handlers: TokenStream = items
    .iter()
    .map(|impl_item: &ImplItem| {
      parse_item_into_handler(impl_item).unwrap_or_else(|e| e.to_compile_error())
    })
    .collect();

  parsed_handlers
}

fn parse_item_into_handler(impl_item: &ImplItem) -> Result<TokenStream, syn::Error> {
  match impl_item {
    ImplItem::Fn(impl_item_fn) => Ok(parse_trait_fn(impl_item_fn)?),
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
      async fn #fname(#(#params),*) #freturn
          #fblock

  };

  eprintln!("TOKENS: {}", new_fn_tokens);

  Ok(new_fn_tokens)
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
      "Expected a single identifier, like #[query] or #[body]",
    ))?;

    match ident.to_string().to_uppercase().as_str() {
      "BODY" => Ok(Extractor::Json),
      "QUERY" => Ok(Extractor::Query),
      _ => Err(syn::Error::new_spanned(
        ident,
        format!("unknown extractor `{ident}`, expected body, query, etc..."),
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
      "Method must have #[http(Get, \"/path\")] attribute",
    )))?;

  attr.parse_args::<Http>().map_err(|e| {
    syn::Error::new_spanned(
      attr,
      format!("Failed to parse http attribute with error {}", e),
    )
  })
}

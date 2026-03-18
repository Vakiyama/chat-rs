use proc_macro2::TokenStream;
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
  let parsed: Vec<TokenStream> = items
    .iter()
    .map(|impl_item: &ImplItem| parse_item(impl_item))
    .collect();

  todo!()
}

fn parse_item(impl_item: &ImplItem) -> TokenStream {
  match impl_item {
    ImplItem::Fn(impl_item_fn) => parse_trait_fn(impl_item_fn),
    _ => syn::Error::new_spanned(impl_item, "Trait item must be a function.").to_compile_error(),
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

fn parse_trait_fn(f: &ImplItemFn) -> TokenStream {
  match find_http_attr(f) {
    Err(e) => e.to_compile_error(),
    Ok(http) => {
      // we need to rebuild the arguments from #[body] request: SomeType, #[query] ... into
      // Json(request): SomeType, Query(query): ...

      f.sig.inputs.iter();
      todo!()
    }
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
    FnArg::Receiver(_) => Ok(quote! { #arg }), // this is the self arg
    FnArg::Typed(pat_type) => {
      let filtered: Vec<&Attribute> = pat_type
        .attrs
        .iter()
        .filter(|attr| attr.style == syn::AttrStyle::Outer)
        .collect();

      let attr = if filtered.len() > 1 {
        Err(syn::Error::new_spanned(
          arg,
          "Must have at most 1 argument annotation, such as \"#[query]\"",
        ))
      } else {
        Ok(&pat_type.attrs[0])
      }?;

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
      }?
      .to_string();

      let ty = pat_type.ty.as_ref();

      // use axum::{Json, extract::State, response::IntoResponse};
      Ok(match extractor {
        Extractor::Json => quote! { axum::Json(#ident_string): axum::Json(#ty) },
        Extractor::Query => quote! { axum::Query(#ident_string): axum::Query(#ty) },
      })
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

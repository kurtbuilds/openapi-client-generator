use convert_case::{Case, Casing};
use openapiv3::{OpenAPI, ReferenceOr, Schema, SchemaKind};
use proc_macro2::{Ident, Span, TokenStream};
use quote::{quote, ToTokens};


pub trait ToToken {
    fn to_token(&self, spec: &OpenAPI) -> TokenStream;
}

impl ToToken for Schema {
    fn to_token(&self, spec: &OpenAPI) -> TokenStream {
        let z = match &self.schema_kind {
            SchemaKind::Type(openapiv3::Type::String(s)) => quote!(String),
            SchemaKind::Type(openapiv3::Type::Number(_)) => quote!(f64),
            SchemaKind::Type(openapiv3::Type::Integer(_)) => quote!(i64),
            SchemaKind::Type(openapiv3::Type::Boolean{}) => quote!(bool),
            SchemaKind::Type(openapiv3::Type::Object(o)) => {
                quote!(serde_json::Value)
            }
            SchemaKind::Type(openapiv3::Type::Array(a)) => {
                let inside = a.items
                    .as_ref()
                    .unwrap()
                    .unbox()
                    .to_token(spec);
                quote! { Vec<#inside> }
            }
            SchemaKind::Any(..) => quote!(serde_json::Value),
            SchemaKind::AllOf{..} => quote!(serde_json::Value),
            SchemaKind::OneOf{..} => quote!(serde_json::Value),
            SchemaKind::AnyOf { .. } => quote!(serde_json::Value),
            _ => {
                println!("unimplemented: {:#?}", self);
                unimplemented!()
            },
        };
        if self.schema_data.nullable {
            quote! { Option<#z> }
        } else {
            z
        }
    }
}


pub fn get_struct_name(reference: &str) -> Option<String> {
    let mut parts = reference.split('/');
    if parts.next() != Some("#") {
        return None;
    }
    if parts.next() != Some("components") {
        return None;
    }
    if parts.next() != Some("schemas") {
        return None;
    }
    parts.next().map(|s| s.to_case(Case::Pascal))
}


impl ToToken for ReferenceOr<Schema> {
    fn to_token(&self, spec: &OpenAPI) -> TokenStream {
        match self {
            ReferenceOr::Reference{ reference } => {
                let name = get_struct_name(&reference).unwrap();
                syn::Ident::new(&name, Span::call_site()).to_token_stream()
            }
            ReferenceOr::Item(s) => s.to_token(spec),
        }
    }
}


pub trait ToIdent {
    fn to_struct_name(&self) -> syn::Ident;
    fn to_ident(&self) -> syn::Ident;
    fn is_restricted(&self) -> bool;
    fn serde_rename(&self) -> Option<TokenStream>;
}

fn sanitize(s: &str) -> String  {
    // custom logic for Github openapi spec lol
    if s == "+1" {
        return "PlusOne".to_string()
    } else if s == "-1" {
        return "MinusOne".to_string()
    }
    s
        .replace("/", "_")
        .replace("@", "")
        .replace("'", "")
        .replace("+", "")
}

fn assert_valid_ident(s: &str, original: &str) {
    if s.chars().all(|c| c.is_numeric()) {
        panic!("Numeric identifier: {}", original)
    }
}

impl ToIdent for str {
    fn to_struct_name(&self) -> syn::Ident {
        let s = sanitize(self);
        let mut s = s.to_case(Case::Pascal);
        if s.is_restricted() {
            s += "Struct"
        }
        assert_valid_ident(&s, self);
        syn::Ident::new(&s, Span::call_site())
    }

    fn to_ident(&self) -> Ident {
        let s = sanitize(self);
        let mut s = s.to_case(Case::Snake);
        if s.is_restricted() {
            s += "_"
        }
        assert_valid_ident(&s, self);
        syn::Ident::new(&s, Span::call_site())
    }

    fn is_restricted(&self) -> bool {
        ["type", "use", "ref"].contains(&self)
    }

    fn serde_rename(&self) -> Option<TokenStream> {
        if self.is_restricted()
            || self.chars().next().unwrap().is_numeric()
            || self.contains('@')
            || self.contains('+')
        {
            Some(quote! {
                #[serde(rename = #self)]
            })
        } else {
            None
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_ident() {
        assert_eq!("meta/root".to_ident(), "meta_root");
    }
}
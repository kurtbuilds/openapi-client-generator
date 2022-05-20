use convert_case::{Case, Casing};
use openapiv3::{OpenAPI, Operation, Parameter, ReferenceOr, RequestBody, Schema, SchemaKind, StatusCode};
use proc_macro2::TokenStream;
use quote::quote;
use serde::de::Unexpected::Option;
use crate::codegen::util;
use crate::codegen::util::ToToken;
use crate::codegen::util::ToIdent;


/// Generates the client code for a given OpenAPI yaml specification.
pub fn generate_lib_rs(spec: &OpenAPI, name: &str) -> TokenStream {
    println!("Generating client for {}", name);
    let struct_Client = struct_Client(name);
    println!("generated struct");
    let impl_Client = impl_Client(name, spec);
    println!("generated service client");

    let security = if spec.security.is_some() {
        let struct_ServiceAuthentication = struct_ServiceAuthentication(name, spec);
        let impl_ServiceAuthentication = impl_ServiceAuthentication(name, spec);
        let impl_Authenticatable = impl_Authenticatable(name, spec);
        quote! {
            #struct_ServiceAuthentication
            #impl_ServiceAuthentication
            #impl_Authenticatable
        }
    } else {
        quote! {}
    };

    quote! {
        #struct_Client
        #impl_Client
        #security
    }
}

pub fn service_auth_struct_name(service_name: &str) -> syn::Ident {
    quote::format_ident!("{}Authentication", service_name)
}

pub fn service_client_struct_name(service_name: &str) -> syn::Ident {
    quote::format_ident!("{}Client", service_name)
}

pub fn struct_Client(service_name: &str) -> TokenStream {
    let auth_struct_name = service_auth_struct_name(service_name);
    let client_struct_name = service_client_struct_name(service_name);

    quote! {
        pub struct #client_struct_name {
            client: httpclient::Client,
            authentication: Option<#auth_struct_name>,
        }
    }
}


pub fn build_docs(operation: &Operation) -> String {
    let mut doc_pieces = vec![];
    if let Some(summary) = operation.summary.as_ref() {
        if !summary.is_empty() {
            doc_pieces.push(summary.clone());
        }
    }
    if let Some(description) = operation.description.as_ref() {
        if !description.is_empty() {
            if doc_pieces.len() > 0 && description == &doc_pieces[0] {} else {
                doc_pieces.push(description.clone());
            }
        }
    }
    if let Some(external_docs) = operation.external_docs.as_ref() {
        doc_pieces.push(format!("See endpoint docs at <{}>.", external_docs.url));
    }
    doc_pieces.join("\n\n")
}


pub fn build_url(operation: &Operation, path: &str) -> TokenStream {
    if operation.parameters.len() > 0 {
        quote! {
            &format!(#path)
        }
    } else {
        quote! {
            #path
        }
    }
}


pub fn build_method(spec: &OpenAPI, path: &str, method: &str, operation: &Operation) -> core::option::Option<TokenStream> {
    let name = operation.operation_id.as_ref().unwrap().to_case(Case::Snake);

    let path_args: Vec<(String, &Schema)> = operation.parameters.iter().map(|params| {
        let param: &Parameter = params.as_item().unwrap();
        let schema = param.parameter_data_ref().schema().unwrap().resolve(spec);
        (param.parameter_data_ref().name.to_case(Case::Snake), schema)
    }).collect();

    let body_args: std::option::Option<Vec<(String, &Schema)>> = operation.request_body.as_ref().map(|body| {
        let body: &RequestBody = body.as_item().unwrap();
        body.content.iter()
            .filter(|(key, _)| key.as_str() == "application/json")
            .filter_map(|(name, media_type)| {
                let body_schema = media_type.schema
                    .as_ref()
                    .unwrap()
                    .resolve(spec);
                body_schema.properties()
                    .map(|map| map.into_iter())
            })
            .flatten()
            .filter(|(k, v)| !["client_id", "secret"].contains(&k.as_str()))
            .map(|(k, v)| {
                (k.clone(), v.resolve(spec))
            })
            .collect()
    });

    let fn_args: Vec<TokenStream> = path_args.iter().chain(body_args.iter().flatten()).map(|(name, schema)| {
        let k = name.to_ident();
        let tok = schema.to_token(spec);
        quote!(#k: #tok)
    }).collect();

    let json: TokenStream = if let Some(body_args) = body_args {
        let json_fields: Vec<TokenStream> = body_args.iter().map(|(k, _schema)| {
            let iden = k.to_ident();
            quote!(#k: #iden)
        })
            .collect::<Vec<_>>();
        ;
        quote! {
                    .json(json!({
                        #(#json_fields),*
                    }))
                }
    } else {
        quote! {}
    };

    let name = syn::Ident::new(&name, proc_macro2::Span::call_site());
    let method = syn::Ident::new(method, proc_macro2::Span::call_site());
    let response_success = operation.responses.responses
        .get(&StatusCode::Code(200))
        .unwrap()
        .as_item()
        .unwrap();
    let response_success_struct: TokenStream = match response_success.content.get("application/json") {
        Some(r) => r.schema.as_ref().unwrap().to_token(spec),
        None => return None,
    };
    let docstring: String = build_docs(operation);
    let url: TokenStream = build_url(operation, &path);

    Some(quote! {
        #[doc = #docstring]
        pub async fn #name(&self, #(#fn_args),*) -> anyhow::Result<#response_success_struct> {
             let res = self.client.#method(#url)
                #json
                .authenticate(&self.authentication)
                .send()
                .await
                .unwrap()
                .error_for_status();
            match res {
                Ok(res) => res
                    .json()
                    .await
                    .map_err(|e| anyhow::anyhow!("{:?}", e)),
                Err(res) => {
                    let text = res
                        .text()
                        .await
                        .map_err(|e| anyhow::anyhow!("{:?}", e));
                    Err(anyhow::anyhow!("{:?}", text))
                }
            }
        }
    })
}


pub fn impl_ServiceClient_paths(spec: &OpenAPI) -> impl Iterator<Item=TokenStream> + '_ {
    let mut it = spec.paths.iter();
    spec.paths.iter().map(|(path, item)| {
        let item = item.as_item().unwrap();
        vec![
            (item.get.as_ref(), "get"),
            (item.post.as_ref(), "post"),
            (item.delete.as_ref(), "delete"),
            (item.put.as_ref(), "put"),
        ].into_iter()
            .filter_map(|x| {
            if let Some(operation) = x.0 {
                build_method(spec, path, x.1, operation)
            } else {
                None
            }
        })
    })
        .flatten()
}


pub fn impl_Client(service_name: &str, spec: &OpenAPI) -> TokenStream {
    let client_struct_name = service_client_struct_name(service_name);
    let auth_struct_name = service_auth_struct_name(service_name);
    let path_fns = impl_ServiceClient_paths(spec);

    quote! {
        impl #client_struct_name {
            pub fn new(url: &str) -> Self {
                let client = httpclient::Client::new(Some(url.to_string()));
                let authentication = None;
                Self {
                    client,
                    authentication,
                }
            }

            pub fn with_authentication(mut self, authentication: #auth_struct_name) -> Self {
                self.authentication = Some(authentication);
                self
            }

            pub fn with_middleware<M: httpclient::Middleware + 'static>(mut self, middleware: M) -> Self {
                self.client = self.client.with_middleware(middleware);
                self
            }

            #(#path_fns)*
        }
    }
}

pub fn struct_ServiceAuthentication(service_name: &str, spec: &OpenAPI) -> TokenStream {
    let auth_struct_name = service_auth_struct_name(service_name);

    let variants = spec.security.as_ref().unwrap().iter().map(|security| {
        let args = security.iter().map(|(k, scopes)| {
            k.to_ident()
        });
        let (name, scopes) = security.iter().next().unwrap();
        let variant_name = syn::Ident::new(&name.to_case(Case::Pascal), proc_macro2::Span::call_site());
        quote! {
            #variant_name {
                #(#args: String),*
            }
        }
    });
    quote! {
        pub enum #auth_struct_name {
            #(#variants),*
        }
    }
}

pub fn impl_ServiceAuthentication_from_env(service_name: &str, spec: &OpenAPI) -> TokenStream {
    let first_variant = spec.security.as_ref().unwrap().iter().next().unwrap();
    let fields = first_variant.iter().map(|(k, scopes)| {
        let field = syn::Ident::new(&k.to_case(Case::Snake), proc_macro2::Span::call_site());
        let env_var = if k.to_lowercase().starts_with(&service_name.to_lowercase()) {
            k.to_case(Case::ScreamingSnake)
        } else {
            format!("{}_{}", service_name.to_case(Case::ScreamingSnake), k.to_case(Case::ScreamingSnake))
        };
        let expect = format!("Environment variable {} is not set.", env_var);
        quote! {
            #field: std::env::var(#env_var).expect(#expect)
        }
    });
    let (name, scopes) = first_variant.iter().next().unwrap();
    let variant_name = syn::Ident::new(&name.to_case(Case::Pascal), proc_macro2::Span::call_site());
    quote! {
        pub fn from_env() -> Self {
            Self::#variant_name {
                #(#fields),*
            }
        }
    }
}


pub fn impl_ServiceAuthentication(service_name: &str, spec: &OpenAPI) -> TokenStream {
    let auth_struct_name = service_auth_struct_name(service_name);
    let from_env = impl_ServiceAuthentication_from_env(service_name, spec);

    quote! {
        impl #auth_struct_name {
            #from_env
        }
    }
}

pub fn impl_Authenticatable(service_name: &str, spec: &OpenAPI) -> TokenStream {
    let auth_struct_name = service_auth_struct_name(service_name);
    quote! {
        trait Authenticatable {
            fn authenticate(self, authenticator: &Option<#auth_struct_name>) -> Self;
        }

        impl<'a> Authenticatable for RequestBuilder<'a> {
            fn authenticate(self, authenticator: &Option<#auth_struct_name>) -> Self {
                if let Some(authenticator) = authenticator {
                    match authenticator {
                        PlaidAuthentication::ClientId { client_id, secret, ..} => {
                            self.push_json(json!({
                                "client_id": client_id,
                                "secret": secret,
                            }))
                        }
                    }
                } else {
                    self
                }
            }
        }
    }
}
